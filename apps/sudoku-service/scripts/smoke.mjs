// Smoke test: drives the sudoku-service over real WebSocket connections.
// Start the service with a short match first, then run this:
//   MATCH_DURATION_SECS=4 PORT=4111 cargo run   (in one shell)
//   node scripts/smoke.mjs                        (in another)
const BASE = process.env.WS || "ws://localhost:4111/sudoku/ws"

function client(pid) {
  const ws = new WebSocket(`${BASE}?pid=${pid}`)
  const inbox = [] // received-but-unconsumed messages
  const waiters = [] // pending next() calls: { pred, resolve, timer }

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

/** First empty cell in the givens grid. */
function firstEmpty(givens) {
  for (let r = 0; r < 9; r++) for (let c = 0; c < 9; c++) if (givens[r][c] === 0) return [r, c]
  throw new Error("no empty cell")
}

/** First clue cell in the givens grid. */
function firstGiven(givens) {
  for (let r = 0; r < 9; r++) for (let c = 0; c < 9; c++) if (givens[r][c] !== 0) return [r, c]
  throw new Error("no given cell")
}

async function main() {
  // --- Create + join: anonymous two-player game --------------------------------
  const a = client("alice")
  await a.open()
  a.send({ type: "create", difficulty: "easy", name: "Alice" })
  const aJoined = await a.next((m) => m.type === "joined")
  assert(aJoined.seat === "p1", "creator seated as p1")
  const code = aJoined.code
  console.log(`  room code: ${code}`)

  const b = client("bob")
  await b.open()
  b.send({ type: "join", code, name: "Bob" })
  const bJoined = await b.next((m) => m.type === "joined")
  assert(bJoined.seat === "p2", "second player seated as p2")

  const aPlaying = await a.next((m) => m.type === "state" && m.status === "playing")
  const bPlaying = await b.next((m) => m.type === "state" && m.status === "playing")
  assert(typeof aPlaying.deadline === "number" && aPlaying.deadline > Date.now(), "deadline is set on start")
  assert(
    JSON.stringify(aPlaying.givens) === JSON.stringify(bPlaying.givens),
    "both players get the same givens",
  )

  // --- Redaction: opponent view is a boolean mask, never digits ----------------
  const [er, ec] = firstEmpty(aPlaying.givens)
  a.drain()
  b.drain()
  a.send({ type: "set_cell", row: er, col: ec, value: 5 })

  const aState = await a.next((m) => m.type === "state")
  assert(aState.board[er][ec] === 5, "own set_cell reflected in own board")

  const bState = await b.next((m) => m.type === "state")
  assert(bState.oppFilled[er][ec] === true, "opponent sees the cell as filled")
  assert(typeof bState.oppFilled[er][ec] === "boolean", "opponent fill is a boolean, not a digit")
  assert(bState.board[er][ec] === 0, "opponent's own board is unchanged (no leak)")
  assert(bState.oppBoard === undefined, "no opponent board/digits field exists")
  assert(typeof bState.oppCorrect === "number", "opponent correct count is a number only")

  // --- Authority: cannot overwrite a clue --------------------------------------
  const [gr, gc] = firstGiven(aPlaying.givens)
  a.send({ type: "set_cell", row: gr, col: gc, value: 1 })
  const err = await a.next((m) => m.type === "error")
  assert(err.code === "given_cell", "writing a clue cell is rejected")

  // --- Matchmaking: two searchers get the same room code -----------------------
  const c = client("carol")
  const d = client("dave")
  await c.open()
  await d.open()
  c.send({ type: "find_match", name: "Carol", difficulty: "medium" })
  d.send({ type: "find_match", name: "Dave", difficulty: "medium" })
  const cFound = await c.next((m) => m.type === "match_found")
  const dFound = await d.next((m) => m.type === "match_found")
  assert(cFound.code === dFound.code, "matchmaking pairs both into one room")

  // --- Timer expiry: no moves → draw when the clock runs out -------------------
  const e = client("erin")
  const f = client("finn")
  await e.open()
  await f.open()
  e.send({ type: "create", difficulty: "easy", name: "Erin" })
  const eJoined = await e.next((m) => m.type === "joined")
  f.send({ type: "join", code: eJoined.code, name: "Finn" })
  await f.next((m) => m.type === "joined")
  await e.next((m) => m.type === "state" && m.status === "playing")

  // Neither moves; wait for the countdown (MATCH_DURATION_SECS) to elapse.
  const eOver = await e.next((m) => m.type === "game_over", 10000)
  const fOver = await f.next((m) => m.type === "game_over", 10000)
  assert(eOver.winner === "draw", "no moves by either player → draw")
  assert(fOver.winner === "draw", "draw is symmetric for both players")
  assert(eOver.yourCorrect === eOver.oppCorrect, "equal correct counts on a draw")

  console.log("\nAll sudoku smoke checks passed ✔")
  ;[a, b, c, d, e, f].forEach((cl) => cl.close())
  process.exit(0)
}

main().catch((err) => {
  console.error(err)
  process.exit(1)
})
