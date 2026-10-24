//! A single Sudoku match: two seats, a shared puzzle, a countdown, and the
//! redacted per-player broadcast. Ported from
//! `connect-four-service/internal/room/room.go` (actor model, identity-based
//! reconnect, empty-room grace), adapted to the timed race-to-most-correct game.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

use crate::game::{self, Difficulty, Grid};
use crate::protocol::{Identity, Outbound, PlayerInfo};

use super::manager::Manager;

/// How long a room with no connected players is kept alive so a refreshing
/// player can reconnect to the same seat before it is reclaimed.
const EMPTY_ROOM_GRACE: Duration = Duration::from_secs(60);

/// The two seats. Sudoku has no colours, so seats are just p1/p2.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Seat {
    P1,
    P2,
}

impl Seat {
    fn as_str(self) -> &'static str {
        match self {
            Seat::P1 => "p1",
            Seat::P2 => "p2",
        }
    }
    fn other(self) -> Seat {
        match self {
            Seat::P1 => Seat::P2,
            Seat::P2 => Seat::P1,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Waiting,
    Playing,
    Over,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Waiting => "waiting",
            Status::Playing => "playing",
            Status::Over => "over",
        }
    }
}

/// Who won a finished game.
#[derive(Clone, Copy)]
enum GameWinner {
    Seat(Seat),
    Draw,
}

/// The room's view of one connection. `ws::client_loop` builds these.
#[derive(Clone)]
pub struct Player {
    pub conn_id: u64,
    pub identity: Identity,
    tx: mpsc::Sender<Outbound>,
    close_tx: Arc<watch::Sender<bool>>,
}

impl Player {
    pub fn new(
        conn_id: u64,
        identity: Identity,
        tx: mpsc::Sender<Outbound>,
        close_tx: Arc<watch::Sender<bool>>,
    ) -> Player {
        Player {
            conn_id,
            identity,
            tx,
            close_tx,
        }
    }

    /// Queue a message for delivery. Never blocks the room task: a client whose
    /// buffer is full is simply dropped (its write task will end).
    pub fn send(&self, msg: Outbound) {
        let _ = self.tx.try_send(msg);
    }

    /// Signal both of this connection's loops to shut down.
    pub fn close(&self) {
        let _ = self.close_tx.send(true);
    }
}

/// Commands posted to a room's task.
pub enum Command {
    Join { player: Player, name: String },
    SetCell { player: Player, row: usize, col: usize, value: u8 },
    ClearCell { player: Player, row: usize, col: usize },
    Rematch { player: Player },
    Leave { player: Player },
}

/// A cloneable handle to a room's command channel.
#[derive(Clone)]
pub struct RoomHandle {
    pub code: String,
    tx: mpsc::Sender<Command>,
}

impl RoomHandle {
    pub(super) fn new(code: String, tx: mpsc::Sender<Command>) -> RoomHandle {
        RoomHandle { code, tx }
    }

    pub async fn join(&self, player: Player, name: String) {
        let _ = self.tx.send(Command::Join { player, name }).await;
    }
    pub async fn set_cell(&self, player: Player, row: usize, col: usize, value: u8) {
        let _ = self.tx.send(Command::SetCell { player, row, col, value }).await;
    }
    pub async fn clear_cell(&self, player: Player, row: usize, col: usize) {
        let _ = self.tx.send(Command::ClearCell { player, row, col }).await;
    }
    pub async fn rematch(&self, player: Player) {
        let _ = self.tx.send(Command::Rematch { player }).await;
    }
    pub async fn leave(&self, player: Player) {
        let _ = self.tx.send(Command::Leave { player }).await;
    }
}

struct SeatState {
    assigned: bool,
    identity: Identity,
    name: String,
    player: Option<Player>, // None when the seat's player is disconnected
    board: Grid,
    rematch: bool,
}

impl SeatState {
    fn new() -> SeatState {
        SeatState {
            assigned: false,
            identity: Identity { kind: String::new(), id: String::new() },
            name: String::new(),
            player: None,
            board: [[0u8; 9]; 9],
            rematch: false,
        }
    }
}

pub struct Room {
    code: String,
    mgr: Arc<Manager>,
    difficulty: Difficulty,
    match_duration: Duration,

    solution: Grid,
    givens: Grid,
    status: Status,
    deadline: Option<Instant>,
    deadline_ms: Option<i64>,
    winner: Option<GameWinner>,
    seats: HashMap<Seat, SeatState>,
    destroy_at: Option<Instant>,
}

impl Room {
    pub fn new(code: String, mgr: Arc<Manager>, difficulty: Difficulty) -> Room {
        let match_duration = std::env::var("MATCH_DURATION_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(300));

        let mut seats = HashMap::new();
        seats.insert(Seat::P1, SeatState::new());
        seats.insert(Seat::P2, SeatState::new());

        Room {
            code,
            mgr,
            difficulty,
            match_duration,
            solution: [[0u8; 9]; 9],
            givens: [[0u8; 9]; 9],
            status: Status::Waiting,
            deadline: None,
            deadline_ms: None,
            winner: None,
            seats,
            destroy_at: None,
        }
    }

    /// The room's event loop. Serially handles commands and the two timers
    /// (match deadline, empty-room reclaim).
    pub async fn run(mut self, mut rx: mpsc::Receiver<Command>) {
        // A freshly created room (e.g. one minted for a queue match) starts with
        // no players; reclaim it if nobody ever joins.
        self.schedule_destroy();

        loop {
            let deadline_at = sleep_until_opt(self.deadline);
            let destroy_at = sleep_until_opt(self.destroy_at);

            tokio::select! {
                maybe = rx.recv() => {
                    match maybe {
                        Some(cmd) => self.handle(cmd),
                        None => break,
                    }
                }
                _ = deadline_at, if self.deadline.is_some() && self.status == Status::Playing => {
                    self.finish();
                }
                _ = destroy_at, if self.destroy_at.is_some() => {
                    if self.destroy_if_empty() {
                        break;
                    }
                }
            }
        }
    }

    fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::Join { player, name } => self.handle_join(player, name),
            Command::SetCell { player, row, col, value } => self.handle_set_cell(player, row, col, value),
            Command::ClearCell { player, row, col } => self.handle_clear_cell(player, row, col),
            Command::Rematch { player } => self.handle_rematch(player),
            Command::Leave { player } => self.handle_leave(player),
        }
    }

    fn handle_join(&mut self, player: Player, name: String) {
        self.cancel_destroy();
        let id = player.identity.clone();

        // Reconnect: same identity already owns a seat.
        let reconnect_seat = self.seats.iter().find_map(|(s, st)| {
            if st.assigned && st.identity == id {
                Some(*s)
            } else {
                None
            }
        });
        if let Some(s) = reconnect_seat {
            let st = self.seats.get_mut(&s).unwrap();
            let old = st.player.take();
            st.player = Some(player.clone());
            if !name.is_empty() {
                st.name = name;
            }
            if let Some(old) = old {
                if old.conn_id != player.conn_id {
                    old.close(); // a second tab takes over the seat
                }
            }
            player.send(Outbound::Joined {
                seat: s.as_str().to_string(),
                code: self.code.clone(),
                you: id,
            });
            if let Some(op) = self.seats[&s.other()].player.clone() {
                op.send(Outbound::OpponentReconnected);
            }
            self.broadcast_state();
            return;
        }

        // New player: take the first free seat (p1, then p2).
        for s in [Seat::P1, Seat::P2] {
            let st = self.seats.get_mut(&s).unwrap();
            if !st.assigned {
                st.assigned = true;
                st.identity = id.clone();
                st.name = name;
                st.player = Some(player.clone());
                player.send(Outbound::Joined {
                    seat: s.as_str().to_string(),
                    code: self.code.clone(),
                    you: id,
                });
                if self.seats[&Seat::P1].assigned
                    && self.seats[&Seat::P2].assigned
                    && self.status == Status::Waiting
                {
                    self.start_game();
                }
                self.broadcast_state();
                return;
            }
        }

        // Both seats belong to other identities.
        player.send(Outbound::error("room_full", "This game already has two players."));
        player.close();
    }

    fn start_game(&mut self) {
        let puzzle = game::generate(self.difficulty, &mut rand::thread_rng());
        self.solution = puzzle.solution;
        self.givens = puzzle.givens;
        for st in self.seats.values_mut() {
            st.board = puzzle.givens;
            st.rematch = false;
        }
        self.status = Status::Playing;
        self.winner = None;
        self.deadline = Some(Instant::now() + self.match_duration);
        self.deadline_ms = Some(now_unix_ms() + self.match_duration.as_millis() as i64);
    }

    fn handle_set_cell(&mut self, player: Player, row: usize, col: usize, value: u8) {
        let s = match self.seat_of(&player) {
            Some(s) => s,
            None => {
                player.send(Outbound::error("invalid", "You are not a player in this game."));
                return;
            }
        };
        if self.status != Status::Playing {
            player.send(Outbound::error("invalid", "The game is not in progress."));
            return;
        }
        if row >= 9 || col >= 9 || !(1..=9).contains(&value) {
            player.send(Outbound::error("invalid", "That move is out of range."));
            return;
        }
        if self.givens[row][col] != 0 {
            player.send(Outbound::error("given_cell", "That cell is a fixed clue."));
            return;
        }
        self.seats.get_mut(&s).unwrap().board[row][col] = value;
        if game::is_complete_correct(&self.seats[&s].board, &self.solution) {
            self.finish();
        } else {
            self.broadcast_state();
        }
    }

    fn handle_clear_cell(&mut self, player: Player, row: usize, col: usize) {
        let s = match self.seat_of(&player) {
            Some(s) => s,
            None => return,
        };
        if self.status != Status::Playing || row >= 9 || col >= 9 {
            return;
        }
        if self.givens[row][col] != 0 {
            return;
        }
        self.seats.get_mut(&s).unwrap().board[row][col] = 0;
        self.broadcast_state();
    }

    /// End the match: winner is whoever has more correct cells (tie = draw).
    fn finish(&mut self) {
        let c1 = game::correct_count(&self.seats[&Seat::P1].board, &self.solution);
        let c2 = game::correct_count(&self.seats[&Seat::P2].board, &self.solution);
        let winner = if c1 > c2 {
            GameWinner::Seat(Seat::P1)
        } else if c2 > c1 {
            GameWinner::Seat(Seat::P2)
        } else {
            GameWinner::Draw
        };
        self.status = Status::Over;
        self.winner = Some(winner);
        self.deadline = None;
        self.broadcast_state();

        // One-shot game_over, from each recipient's perspective.
        for s in [Seat::P1, Seat::P2] {
            if let Some(p) = self.seats[&s].player.clone() {
                let (yours, opp) = if s == Seat::P1 { (c1, c2) } else { (c2, c1) };
                p.send(Outbound::GameOver {
                    winner: self.winner_str(winner, s),
                    your_correct: yours,
                    opp_correct: opp,
                });
            }
        }
    }

    fn handle_rematch(&mut self, player: Player) {
        if self.status != Status::Over {
            return;
        }
        let s = match self.seat_of(&player) {
            Some(s) => s,
            None => return,
        };
        self.seats.get_mut(&s).unwrap().rematch = true;
        if let Some(op) = self.seats[&s.other()].player.clone() {
            op.send(Outbound::RematchOffered { by: s.as_str().to_string() });
        }
        if self.seats[&Seat::P1].rematch && self.seats[&Seat::P2].rematch {
            self.start_game();
            for st in self.seats.values() {
                if let Some(p) = &st.player {
                    p.send(Outbound::RematchStarted);
                }
            }
            self.broadcast_state();
        }
    }

    fn handle_leave(&mut self, player: Player) {
        let s = match self.seat_of(&player) {
            Some(s) => s,
            None => return,
        };
        self.seats.get_mut(&s).unwrap().player = None;
        if self.status == Status::Playing {
            if let Some(op) = self.seats[&s.other()].player.clone() {
                op.send(Outbound::OpponentDisconnected);
            }
        }
        self.broadcast_state();
        if self.connected_count() == 0 {
            self.schedule_destroy();
        }
    }

    fn seat_of(&self, player: &Player) -> Option<Seat> {
        self.seats.iter().find_map(|(s, st)| match &st.player {
            Some(p) if p.conn_id == player.conn_id => Some(*s),
            _ => None,
        })
    }

    fn connected_count(&self) -> usize {
        self.seats.values().filter(|st| st.player.is_some()).count()
    }

    fn winner_str(&self, winner: GameWinner, viewer: Seat) -> String {
        match winner {
            GameWinner::Draw => "draw".to_string(),
            GameWinner::Seat(w) => if w == viewer { "you" } else { "opponent" }.to_string(),
        }
    }

    fn broadcast_state(&self) {
        let playing_or_over = self.status != Status::Waiting;

        for s in [Seat::P1, Seat::P2] {
            let st = &self.seats[&s];
            let player = match &st.player {
                Some(p) => p.clone(),
                None => continue,
            };

            let opp = &self.seats[&s.other()];
            let your_correct = if playing_or_over {
                game::correct_count(&st.board, &self.solution)
            } else {
                0
            };
            let opp_correct = if playing_or_over && opp.assigned {
                game::correct_count(&opp.board, &self.solution)
            } else {
                0
            };
            let opp_filled = if opp.assigned {
                mask_to_vec(&game::filled_mask(&opp.board))
            } else {
                vec![vec![false; 9]; 9]
            };

            let winner = if self.status == Status::Over {
                self.winner.map(|w| self.winner_str(w, s))
            } else {
                None
            };

            player.send(Outbound::State {
                status: self.status.as_str().to_string(),
                difficulty: self.difficulty.as_str().to_string(),
                givens: grid_to_vec(&self.givens),
                board: grid_to_vec(&st.board),
                your_correct,
                opp_filled,
                opp_correct,
                deadline: if self.status == Status::Playing { self.deadline_ms } else { None },
                winner,
                players: self.players_map(),
            });
        }
    }

    fn players_map(&self) -> HashMap<String, PlayerInfo> {
        let mut m = HashMap::new();
        for s in [Seat::P1, Seat::P2] {
            let st = &self.seats[&s];
            if st.assigned {
                m.insert(
                    s.as_str().to_string(),
                    PlayerInfo { name: st.name.clone(), connected: st.player.is_some() },
                );
            }
        }
        m
    }

    fn schedule_destroy(&mut self) {
        self.destroy_at = Some(Instant::now() + EMPTY_ROOM_GRACE);
    }

    fn cancel_destroy(&mut self) {
        self.destroy_at = None;
    }

    /// Returns true if the room removed itself and the task should end.
    fn destroy_if_empty(&mut self) -> bool {
        if self.connected_count() > 0 {
            self.destroy_at = None;
            return false;
        }
        self.mgr.remove(&self.code);
        tracing::info!("sudoku: room {} reclaimed", self.code);
        true
    }
}

fn sleep_until_opt(at: Option<Instant>) -> tokio::time::Sleep {
    let when = at.unwrap_or_else(|| Instant::now() + Duration::from_secs(3600));
    tokio::time::sleep_until(when)
}

fn grid_to_vec(g: &Grid) -> Vec<Vec<u8>> {
    g.iter().map(|row| row.to_vec()).collect()
}

fn mask_to_vec(m: &game::Mask) -> Vec<Vec<bool>> {
    m.iter().map(|row| row.to_vec()).collect()
}

fn now_unix_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
