// Package room owns authoritative game state. Each Room runs a single
// goroutine that serially processes commands posted to it, so room state needs
// no locks. Connections (clients) interact with a room only by posting through
// the exported methods below.
package room

import (
	"log"
	"time"

	"github.com/midtone/connect-four-service/internal/game"
	"github.com/midtone/connect-four-service/internal/protocol"
)

// emptyRoomGrace is how long a room with no connected players is kept alive so
// a refreshing player can reconnect to the same seat before it is reclaimed.
const emptyRoomGrace = 60 * time.Second

// Player is the room's view of a connection. ws.Client implements it. Send
// marshals and delivers a message; it must never block the room goroutine.
type Player interface {
	Identity() protocol.Identity
	Send(v any)
	Close()
}

type seat string

const (
	seatRed    seat = "red"
	seatYellow seat = "yellow"
)

func (s seat) cell() game.Cell {
	if s == seatRed {
		return game.Red
	}
	return game.Yellow
}

func (s seat) other() seat {
	if s == seatRed {
		return seatYellow
	}
	return seatRed
}

type seatState struct {
	assigned bool
	identity protocol.Identity
	name     string
	player   Player // nil when the seat's player is disconnected
	rematch  bool
}

// Room is a single Connect Four game (two seats + spectators rejected).
type Room struct {
	code string
	mgr  *Manager

	cmds chan func()
	done chan struct{}

	board        game.Board
	turn         seat
	status       string // waiting|playing|won|draw
	winner       seat
	winningCells [][2]int
	seats        map[seat]*seatState

	destroyTimer *time.Timer
	closed       bool
}

func newRoom(code string, mgr *Manager) *Room {
	return &Room{
		code:   code,
		mgr:    mgr,
		cmds:   make(chan func(), 32),
		done:   make(chan struct{}),
		turn:   seatRed,
		status: "waiting",
		seats: map[seat]*seatState{
			seatRed:    {},
			seatYellow: {},
		},
	}
}

func (r *Room) Code() string { return r.code }

func (r *Room) run() {
	// A freshly created room (e.g. one minted for a queue match) starts with no
	// players; reclaim it if nobody ever joins.
	r.scheduleDestroy()
	for {
		select {
		case fn := <-r.cmds:
			fn()
		case <-r.done:
			return
		}
	}
}

// post runs fn on the room goroutine. It is a no-op once the room is destroyed.
func (r *Room) post(fn func()) {
	select {
	case r.cmds <- fn:
	case <-r.done:
	}
}

// Join attaches a player (assigning a seat or reconnecting to an existing one).
func (r *Room) Join(p Player, name string) { r.post(func() { r.handleJoin(p, name) }) }

// Move plays a disc into col on behalf of p.
func (r *Room) Move(p Player, col int) { r.post(func() { r.handleMove(p, col) }) }

// Rematch records p's request to play again.
func (r *Room) Rematch(p Player) { r.post(func() { r.handleRematch(p) }) }

// Leave detaches p (disconnect or explicit leave).
func (r *Room) Leave(p Player) { r.post(func() { r.handleLeave(p) }) }

func (r *Room) handleJoin(p Player, name string) {
	r.cancelDestroy()
	id := p.Identity()

	// Reconnect: same identity already owns a seat.
	for s, st := range r.seats {
		if st.assigned && st.identity == id {
			old := st.player
			st.player = p
			if name != "" {
				st.name = name
			}
			if old != nil && old != p {
				old.Close() // a second tab takes over the seat
			}
			p.Send(protocol.Joined{Type: "joined", Seat: string(s), Code: r.code, You: id})
			if op := r.seats[s.other()]; op.player != nil {
				op.player.Send(protocol.Simple{Type: "opponent_reconnected"})
			}
			r.broadcastState()
			return
		}
	}

	// New player: take the first free seat (red, then yellow).
	for _, s := range []seat{seatRed, seatYellow} {
		st := r.seats[s]
		if !st.assigned {
			st.assigned = true
			st.identity = id
			st.name = name
			st.player = p
			p.Send(protocol.Joined{Type: "joined", Seat: string(s), Code: r.code, You: id})
			if r.seats[seatRed].assigned && r.seats[seatYellow].assigned && r.status == "waiting" {
				r.status = "playing"
			}
			r.broadcastState()
			return
		}
	}

	// Both seats belong to other identities.
	p.Send(protocol.Error{Type: "error", Code: "room_full", Message: "This game already has two players."})
	p.Close()
}

func (r *Room) handleMove(p Player, col int) {
	s, ok := r.seatOf(p)
	if !ok {
		p.Send(protocol.Error{Type: "error", Code: "invalid", Message: "You are not a player in this game."})
		return
	}
	if r.status != "playing" {
		p.Send(protocol.Error{Type: "error", Code: "invalid", Message: "The game is not in progress."})
		return
	}
	if s != r.turn {
		p.Send(protocol.Error{Type: "error", Code: "not_your_turn", Message: "It is not your turn."})
		return
	}
	row, placed := r.board.ApplyMove(col, s.cell())
	if !placed {
		p.Send(protocol.Error{Type: "error", Code: "illegal_move", Message: "That column is full or invalid."})
		return
	}
	if win := r.board.DetectWin(row, col); win != nil {
		r.status = "won"
		r.winner = s
		r.winningCells = win
	} else if r.board.IsFull() {
		r.status = "draw"
	} else {
		r.turn = s.other()
	}
	r.broadcastState()
}

func (r *Room) handleRematch(p Player) {
	if r.status != "won" && r.status != "draw" {
		return
	}
	s, ok := r.seatOf(p)
	if !ok {
		return
	}
	r.seats[s].rematch = true
	if op := r.seats[s.other()]; op.player != nil {
		op.player.Send(protocol.RematchOffered{Type: "rematch_offered", By: string(s)})
	}
	if r.seats[seatRed].rematch && r.seats[seatYellow].rematch {
		r.board = game.Board{}
		r.turn = seatRed
		r.status = "playing"
		r.winner = ""
		r.winningCells = nil
		r.seats[seatRed].rematch = false
		r.seats[seatYellow].rematch = false
		for _, st := range r.seats {
			if st.player != nil {
				st.player.Send(protocol.Simple{Type: "rematch_started"})
			}
		}
		r.broadcastState()
	}
}

func (r *Room) handleLeave(p Player) {
	s, ok := r.seatOf(p)
	if !ok {
		return
	}
	r.seats[s].player = nil
	if op := r.seats[s.other()]; op.player != nil && r.status == "playing" {
		op.player.Send(protocol.Simple{Type: "opponent_disconnected"})
	}
	r.broadcastState()
	if r.connectedCount() == 0 {
		r.scheduleDestroy()
	}
}

// seatOf returns the seat currently held by the live player p.
func (r *Room) seatOf(p Player) (seat, bool) {
	for s, st := range r.seats {
		if st.player == p {
			return s, true
		}
	}
	return "", false
}

func (r *Room) connectedCount() int {
	n := 0
	for _, st := range r.seats {
		if st.player != nil {
			n++
		}
	}
	return n
}

func (r *Room) broadcastState() {
	players := make(map[string]protocol.PlayerInfo, 2)
	for s, st := range r.seats {
		if st.assigned {
			players[string(s)] = protocol.PlayerInfo{Name: st.name, Connected: st.player != nil}
		}
	}
	msg := protocol.State{
		Type:         "state",
		Board:        r.board,
		Turn:         string(r.turn),
		Status:       r.status,
		Winner:       string(r.winner),
		WinningCells: r.winningCells,
		Players:      players,
	}
	for _, st := range r.seats {
		if st.player != nil {
			st.player.Send(msg)
		}
	}
}

func (r *Room) scheduleDestroy() {
	r.cancelDestroy()
	r.destroyTimer = time.AfterFunc(emptyRoomGrace, func() {
		r.post(r.destroyIfEmpty)
	})
}

func (r *Room) cancelDestroy() {
	if r.destroyTimer != nil {
		r.destroyTimer.Stop()
		r.destroyTimer = nil
	}
}

func (r *Room) destroyIfEmpty() {
	if r.closed || r.connectedCount() > 0 {
		return
	}
	r.closed = true
	r.mgr.remove(r.code)
	close(r.done)
	log.Printf("connect-four: room %s reclaimed", r.code)
}
