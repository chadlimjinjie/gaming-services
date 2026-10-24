// Smoke test: drives the connect-four-service over real WebSocket connections.
// Run the service on :4110 first, then: node scripts/smoke.mjs
const BASE = process.env.WS || "ws://localhost:4110/connect-four/ws"

function client(pid) {
  const ws = new WebSocket(`${BASE}?pid=${pid}`)
  const inbox = [] // received-but-unconsumed messages
  const waiters = [] // pending next() calls: { pred, resolve, timer }

  // Match buffered messages to pending waiters; each message is consumed by at
  // most one waiter and removed from the inbox once delivered (so a later
  // next() can never re-match a stale frame — the source of earlier flakiness).
  const pump = () => {
    for (let wi = 0; wi < waiters.length; wi++) {
      const w = waiters[wi]
      const idx = inbox.findIndex(w.pred)
      if (idx >= 0) {
        const [msg] = inbox.splice(idx, 1)
        clearTimeout(w.timer)
        waiters.splice(wi, 1)
        wi--
        w.resolve(msg)
      }
    }
  }

  ws.onmessage = (e) => {
    inbox.push(JSON.parse(e.data))
    pump()
  }

  return {
    ws,
    open: () => new Promise((res) => (ws.onopen = () => res())),
    send: (o) => ws.send(JSON.stringify(o)),
    drain: () => {
      inbox.length = 0
    },
    // Resolve with the next message matching pred (scanning the buffer first).
    next: (pred = () => true, timeout = 4000) =>
      new Promise((resolve, reject) => {
        const w = { pred, resolve, timer: null }
        w.timer = setTimeout(() => {
          const i = waiters.indexOf(w)
          if (i >= 0) waiters.splice(i, 1)
          reject(new Error("timeout waiting for message"))
        }, timeout)
        waiters.push(w)
        pump()
      }),
    close: () => ws.close(),
  }
}

const assert = (cond, label) => {
  if (!cond) throw new Error(`FAIL: ${label}`)
  console.log(`  ok: ${label}`)
}

async function main() {
  // --- Create + join: anonymous two-player game --------------------------------
  const a = client("alice")
  await a.open()
  a.send({ type: "create", name: "Alice" })
  const aJoined = await a.next((m) => m.type === "joined")
  assert(aJoined.seat === "red", "creator seated as red")
  const code = aJoined.code
  console.log(`  room code: ${code}`)
  await a.next((m) => m.type === "state" && m.status === "waiting")

  const b = client("bob")
  await b.open()
  b.send({ type: "join", code, name: "Bob" })
  const bJoined = await b.next((m) => m.type === "joined")
  assert(bJoined.seat === "yellow", "second player seated as yellow")
  await a.next((m) => m.type === "state" && m.status === "playing")
  console.log("  ok: game started when both seated")

  // --- Authority: reject out-of-turn -------------------------------------------
  b.send({ type: "move", column: 0 })
  const err = await b.next((m) => m.type === "error")
  assert(err.code === "not_your_turn", "out-of-turn move rejected")

  // --- Play a vertical win for red (Alice) in column 3 -------------------------
  // Red col3, Yellow col0, x4 → red connects four vertically.
  // Fully settle each move on BOTH clients before the next so the turn has
  // flipped server-side regardless of proxy latency (moves use separate sockets).
  a.drain()
  b.drain()
  const move = async (p, col) => {
    p.send({ type: "move", column: col })
    await Promise.all([
      a.next((m) => m.type === "state"),
      b.next((m) => m.type === "state"),
    ])
  }
  for (let i = 0; i < 3; i++) {
    await move(a, 3)
    await move(b, 0)
  }
  a.send({ type: "move", column: 3 }) // 4th red disc → win
  const won = await a.next((m) => m.type === "state" && m.status === "won")
  assert(won.winner === "red", "red wins vertically")
  assert(Array.isArray(won.winningCells) && won.winningCells.length >= 4, "winningCells reported")

  // --- room_full: a third identity is rejected ---------------------------------
  const c = client("carol")
  await c.open()
  c.send({ type: "join", code, name: "Carol" })
  const full = await c.next((m) => m.type === "error")
  assert(full.code === "room_full", "third player rejected as room_full")

  // --- Rematch: both request → fresh playing board -----------------------------
  // Drain stale broadcast states buffered during the finished game first.
  a.drain()
  b.drain()
  a.send({ type: "rematch" })
  await b.next((m) => m.type === "rematch_offered")
  b.send({ type: "rematch" })
  const restarted = await a.next((m) => m.type === "state" && m.status === "playing")
  assert(restarted.board.flat().every((cell) => cell === ""), "rematch board is empty")

  // --- Reconnect: Bob drops and rejoins same identity → same seat --------------
  b.close()
  await a.next((m) => m.type === "opponent_disconnected")
  const b2 = client("bob")
  await b2.open()
  b2.send({ type: "join", code, name: "Bob" })
  const reJoined = await b2.next((m) => m.type === "joined")
  assert(reJoined.seat === "yellow", "reconnecting player keeps yellow seat")
  await a.next((m) => m.type === "opponent_reconnected")
  console.log("  ok: reconnect restored seat + notified opponent")

  a.close()
  b2.close()

  // --- Random matchmaking: two searchers get paired into one room --------------
  const p1 = client("paula")
  const p2 = client("pete")
  await p1.open()
  await p2.open()
  p1.send({ type: "find_match", name: "Paula" })
  p2.send({ type: "find_match", name: "Pete" })
  const m1 = await p1.next((m) => m.type === "match_found")
  const m2 = await p2.next((m) => m.type === "match_found")
  assert(m1.code === m2.code, "both searchers matched into the same room")
  p1.close()
  p2.close()

  console.log("\nALL PASSED")
  await new Promise((r) => setTimeout(r, 200)) // let sockets close before exit
  process.exit(0)
}

main().catch((e) => {
  console.error(e.message || e)
  process.exit(1)
})
