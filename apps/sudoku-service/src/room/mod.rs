//! Authoritative game state. Each `Room` runs a single tokio task that serially
//! processes commands from an `mpsc` channel, so room state needs no locks — the
//! Rust equivalent of connect-four-service's per-room goroutine + command chan.

mod manager;
mod queue;
#[allow(clippy::module_inception)]
mod room;

pub use manager::Manager;
pub use room::{Player, RoomHandle};
