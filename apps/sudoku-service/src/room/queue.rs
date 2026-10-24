//! Single-slot random matchmaker. The first searcher waits; the next one pairs
//! with it — both are told (via `match_found`) which freshly created room to
//! join. The waiting player's chosen difficulty is used for the match. Mirrors
//! `connect-four-service/internal/room/queue.go`.

use std::sync::{Arc, Mutex};

use crate::game::Difficulty;
use crate::protocol::Outbound;

use super::manager::Manager;
use super::room::Player;

struct Waiting {
    player: Player,
    difficulty: Difficulty,
}

pub struct Queue {
    waiting: Mutex<Option<Waiting>>,
}

impl Queue {
    pub fn new() -> Queue {
        Queue { waiting: Mutex::new(None) }
    }

    /// Enqueue `player`, or pair it with a waiting player into a new room.
    pub fn find(&self, mgr: &Arc<Manager>, player: Player, difficulty: Difficulty) {
        let mut guard = self.waiting.lock().unwrap();
        if let Some(prev) = guard.take() {
            if prev.player.conn_id != player.conn_id {
                drop(guard); // release before creating the room
                let room = mgr.create(prev.difficulty);
                prev.player.send(Outbound::MatchFound { code: room.code.clone() });
                player.send(Outbound::MatchFound { code: room.code.clone() });
                return;
            }
            // The same connection searched again — keep it waiting.
            *guard = Some(prev);
            return;
        }
        *guard = Some(Waiting { player, difficulty });
    }

    /// Drop a connection from the queue if it is the one waiting.
    pub fn remove(&self, conn_id: u64) {
        let mut guard = self.waiting.lock().unwrap();
        if guard.as_ref().map(|w| w.player.conn_id) == Some(conn_id) {
            *guard = None;
        }
    }
}
