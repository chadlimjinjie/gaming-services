//! JSON wire contract shared between the WebSocket clients and this server. It is
//! the Rust mirror of the zod schemas in
//! `apps/web-beta/features/sudoku/types.ts` — keep the two in sync.
//!
//! The redaction guarantee lives here in shape only: `State.board` is the
//! recipient's OWN grid, while the opponent is represented solely by
//! `opp_filled` (a boolean mask) and `opp_correct` (a count). The opponent's
//! digits are never part of any serialized type. `room::Room::broadcast_state`
//! is the single place that builds a per-recipient `State`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Resolved player identity. `kind` is always "anon" (ephemeral localStorage
/// uuid). Reconnection and seating key off this value.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub kind: String,
    pub id: String,
}

/// Public view of a seated player sent inside `State`.
#[derive(Serialize, Clone, Debug)]
pub struct PlayerInfo {
    pub name: String,
    pub connected: bool,
}

/// A message a client sends to the server. Internally tagged on `type`.
#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Inbound {
    Create {
        #[serde(default)]
        difficulty: String,
        #[serde(default)]
        name: String,
    },
    Join {
        code: String,
        #[serde(default)]
        name: String,
    },
    FindMatch {
        #[serde(default)]
        difficulty: Option<String>,
        // Accepted for wire compatibility; the name is applied by the follow-up
        // `join` the client sends after `match_found`.
        #[serde(default)]
        #[allow(dead_code)]
        name: String,
    },
    SetCell {
        row: usize,
        col: usize,
        value: u8,
    },
    ClearCell {
        row: usize,
        col: usize,
    },
    Rematch,
    Leave,
    Ping,
}

/// A message the server sends to a client. Internally tagged on `type`.
#[derive(Serialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Outbound {
    /// Confirms this connection was seated.
    Joined {
        seat: String, // "p1" | "p2"
        code: String,
        you: Identity,
    },
    /// Full authoritative snapshot, built per recipient (see module docs).
    State {
        status: String, // "waiting" | "playing" | "over"
        difficulty: String,
        givens: Vec<Vec<u8>>, // shared clue digits (0 elsewhere)
        board: Vec<Vec<u8>>,  // THIS recipient's own board
        #[serde(rename = "yourCorrect")]
        your_correct: u8,
        #[serde(rename = "oppFilled")]
        opp_filled: Vec<Vec<bool>>, // opponent's fill mask ONLY — never digits
        #[serde(rename = "oppCorrect")]
        opp_correct: u8,
        deadline: Option<i64>, // unix ms; null until playing
        #[serde(skip_serializing_if = "Option::is_none")]
        winner: Option<String>, // "you" | "opponent" | "draw" when over
        players: HashMap<String, PlayerInfo>,
    },
    /// Tells a queued player which room to navigate to.
    MatchFound { code: String },
    /// One-shot notification on the transition to a finished game.
    GameOver {
        winner: String, // "you" | "opponent" | "draw"
        #[serde(rename = "yourCorrect")]
        your_correct: u8,
        #[serde(rename = "oppCorrect")]
        opp_correct: u8,
    },
    /// Opponent requested a rematch.
    RematchOffered { by: String }, // "p1" | "p2"
    OpponentDisconnected,
    OpponentReconnected,
    RematchStarted,
    Pong,
    /// A rejected action. `code` is a stable machine-readable string.
    Error { code: String, message: String },
}

impl Outbound {
    pub fn error(code: &str, message: &str) -> Outbound {
        Outbound::Error {
            code: code.to_string(),
            message: message.to_string(),
        }
    }
}
