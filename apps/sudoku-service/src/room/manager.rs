//! Owns the live rooms and the matchmaking queue. The rooms map is guarded by a
//! mutex; per-room state lives behind each room's own task. Mirrors
//! `connect-four-service/internal/room/manager.go`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use rand::RngCore;
use tokio::sync::mpsc;

use crate::game::Difficulty;

use super::queue::Queue;
use super::room::{Command, Room, RoomHandle};

/// Excludes visually ambiguous characters (0/O, 1/I/L).
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
const CODE_LEN: usize = 6;

pub struct Manager {
    rooms: Mutex<HashMap<String, RoomHandle>>,
    pub queue: Queue,
    next_conn_id: AtomicU64,
}

impl Manager {
    pub fn new() -> Arc<Manager> {
        Arc::new(Manager {
            rooms: Mutex::new(HashMap::new()),
            queue: Queue::new(),
            next_conn_id: AtomicU64::new(1),
        })
    }

    /// A process-unique id for a connection, used for seat identity comparisons.
    pub fn next_conn_id(&self) -> u64 {
        self.next_conn_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Mint a fresh room with a unique code and start its task.
    pub fn create(self: &Arc<Self>, difficulty: Difficulty) -> RoomHandle {
        let mut rooms = self.rooms.lock().unwrap();
        let code = loop {
            let c = random_code();
            if !rooms.contains_key(&c) {
                break c;
            }
        };
        let (tx, rx) = mpsc::channel::<Command>(32);
        let handle = RoomHandle::new(code.clone(), tx);
        let room = Room::new(code.clone(), self.clone(), difficulty);
        tokio::spawn(room.run(rx));
        rooms.insert(code, handle.clone());
        handle
    }

    /// Return the room for a code, if it still exists.
    pub fn get(&self, code: &str) -> Option<RoomHandle> {
        self.rooms.lock().unwrap().get(code).cloned()
    }

    pub fn remove(&self, code: &str) {
        self.rooms.lock().unwrap().remove(code);
    }
}

fn random_code() -> String {
    let mut buf = [0u8; CODE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    buf.iter()
        .map(|b| CODE_ALPHABET[(*b as usize) % CODE_ALPHABET.len()] as char)
        .collect()
}
