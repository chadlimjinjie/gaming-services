//! Bridges WebSocket connections to rooms. Each connection resolves an identity
//! once (from `?pid=`), then runs a read loop that dispatches inbound
//! protocol messages and a write task that drains queued outbound messages and
//! sends keepalive pings. Ported from
//! `connect-four-service/internal/ws/handler.go`.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::{mpsc, watch};

use crate::game::Difficulty;
use crate::protocol::{Identity, Inbound, Outbound};
use crate::room::{Manager, Player, RoomHandle};

const PING_PERIOD: Duration = Duration::from_secs(30);
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const SEND_BUFFER: usize = 32;
const MAX_NAME_LEN: usize = 24;

#[derive(Deserialize)]
pub struct ConnectParams {
    pid: Option<String>,
}

/// Upgrade an HTTP request to a WebSocket game connection. Identity is resolved
/// before the upgrade so a missing identity fails as a plain 400.
pub async fn handler(
    ws: WebSocketUpgrade,
    Query(q): Query<ConnectParams>,
    State(mgr): State<Arc<Manager>>,
) -> impl IntoResponse {
    let identity = match q.pid.filter(|p| !p.is_empty()) {
        Some(id) => Identity { kind: "anon".into(), id },
        None => return (StatusCode::BAD_REQUEST, "missing player identity").into_response(),
    };
    ws.on_upgrade(move |socket| client_loop(socket, identity, mgr))
}

async fn client_loop(socket: WebSocket, identity: Identity, mgr: Arc<Manager>) {
    let (mut sink, mut stream) = socket.split();
    let (out_tx, mut out_rx) = mpsc::channel::<Outbound>(SEND_BUFFER);
    let (close_tx, mut close_rx) = watch::channel(false);
    let conn_id = mgr.next_conn_id();
    let player = Player::new(conn_id, identity, out_tx.clone(), Arc::new(close_tx));

    // Write task: drains outbound messages, pings periodically, and sends a
    // clean close frame when signalled.
    let mut write_close = close_rx.clone();
    let write_task = tokio::spawn(async move {
        let mut ping = tokio::time::interval(PING_PERIOD);
        ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                maybe = out_rx.recv() => match maybe {
                    Some(msg) => {
                        let txt = serde_json::to_string(&msg).unwrap_or_default();
                        if sink.send(Message::Text(txt.into())).await.is_err() {
                            break;
                        }
                    }
                    None => break, // all senders dropped
                },
                _ = ping.tick() => {
                    if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                        break;
                    }
                }
                _ = write_close.changed() => {
                    let _ = sink.send(Message::Close(None)).await;
                    break;
                }
            }
        }
    });

    // Read loop: decodes inbound messages and dispatches them to the room.
    let mut room: Option<RoomHandle> = None;
    loop {
        tokio::select! {
            _ = close_rx.changed() => break,
            res = tokio::time::timeout(IDLE_TIMEOUT, stream.next()) => {
                let item = match res {
                    Ok(item) => item,
                    Err(_) => break, // idle timeout — treat as disconnect
                };
                let msg = match item {
                    Some(Ok(m)) => m,
                    _ => break,
                };
                match msg {
                    Message::Text(t) => {
                        let inbound: Inbound = match serde_json::from_str(t.as_str()) {
                            Ok(i) => i,
                            Err(_) => continue,
                        };
                        if !handle_inbound(inbound, &player, &mgr, &mut room).await {
                            break;
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        }
    }

    // Cleanup: leave the room (or drop from the queue), then end the write task.
    if let Some(r) = &room {
        r.leave(player.clone()).await;
    } else {
        mgr.queue.remove(conn_id);
    }
    player.close();
    drop(out_tx);
    let _ = write_task.await;
}

/// Dispatch one inbound message. Returns false to end the read loop.
async fn handle_inbound(
    inbound: Inbound,
    player: &Player,
    mgr: &Arc<Manager>,
    room: &mut Option<RoomHandle>,
) -> bool {
    match inbound {
        Inbound::Create { difficulty, name } => {
            let r = mgr.create(Difficulty::parse(&difficulty));
            r.join(player.clone(), sanitize_name(&name)).await;
            *room = Some(r);
        }
        Inbound::Join { code, name } => match mgr.get(&code) {
            Some(r) => {
                *room = Some(r.clone());
                r.join(player.clone(), sanitize_name(&name)).await;
            }
            None => player.send(Outbound::error("room_not_found", "That game does not exist.")),
        },
        Inbound::FindMatch { difficulty, .. } => {
            let diff = Difficulty::parse(difficulty.as_deref().unwrap_or("medium"));
            mgr.queue.find(mgr, player.clone(), diff);
        }
        Inbound::SetCell { row, col, value } => {
            if let Some(r) = room {
                r.set_cell(player.clone(), row, col, value).await;
            }
        }
        Inbound::ClearCell { row, col } => {
            if let Some(r) = room {
                r.clear_cell(player.clone(), row, col).await;
            }
        }
        Inbound::Rematch => {
            if let Some(r) = room {
                r.rematch(player.clone()).await;
            }
        }
        Inbound::Ping => player.send(Outbound::Pong),
        Inbound::Leave => return false,
    }
    true
}

fn sanitize_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return "Player".to_string();
    }
    trimmed.chars().take(MAX_NAME_LEN).collect()
}
