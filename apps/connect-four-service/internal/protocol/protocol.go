// Package protocol defines the JSON message contract shared between the
// WebSocket clients and the game server. It is the Go mirror of the zod
// schemas in apps/web-beta/features/connect-four/types.ts — keep the two in
// sync.
package protocol

import "github.com/midtone/connect-four-service/internal/game"

// Inbound is any message a client sends to the server. The Type field
// discriminates which other fields are meaningful.
//
//	join       -> Code, Name
//	find_match  (no extra fields)
//	move       -> Column
//	rematch     (no extra fields)
//	leave       (no extra fields)
//	ping        (no extra fields)
type Inbound struct {
	Type   string `json:"type"`
	Code   string `json:"code,omitempty"`
	Name   string `json:"name,omitempty"`
	Column int    `json:"column,omitempty"`
}

// Identity is the resolved player identity. Kind is always "anon": an
// ephemeral player keyed by a localStorage uuid. Everything downstream —
// seats, reconnection — keys off this value.
type Identity struct {
	Kind string `json:"kind"`
	ID   string `json:"id"`
}

// PlayerInfo is the public view of a seated player sent in State.
type PlayerInfo struct {
	Name      string `json:"name"`
	Connected bool   `json:"connected"`
}

// Joined confirms the connection was seated (or is spectating).
type Joined struct {
	Type string   `json:"type"` // "joined"
	Seat string   `json:"seat"` // "red" | "yellow" | "spectator"
	Code string   `json:"code"`
	You  Identity `json:"you"`
}

// State is a full authoritative snapshot of the game, broadcast after every
// accepted move and on every join/leave. Full snapshots (42 cells) keep the
// client trivially in sync and make reconnection a non-event.
type State struct {
	Type         string                 `json:"type"` // "state"
	Board        game.Board             `json:"board"`
	Turn         string                 `json:"turn"`   // "red" | "yellow"
	Status       string                 `json:"status"` // waiting|playing|won|draw
	Winner       string                 `json:"winner,omitempty"`
	WinningCells [][2]int               `json:"winningCells,omitempty"`
	Players      map[string]PlayerInfo  `json:"players"`
}

// MatchFound tells a queued player which room to navigate to.
type MatchFound struct {
	Type string `json:"type"` // "match_found"
	Code string `json:"code"`
}

// RematchOffered notifies the opponent that a rematch was requested.
type RematchOffered struct {
	Type string `json:"type"` // "rematch_offered"
	By   string `json:"by"`   // "red" | "yellow"
}

// Simple covers parameterless server notifications:
// opponent_disconnected, opponent_reconnected, rematch_started, pong.
type Simple struct {
	Type string `json:"type"`
}

// Error reports a rejected action. Code is a stable machine-readable string.
type Error struct {
	Type    string `json:"type"` // "error"
	Code    string `json:"code"` // room_full|not_your_turn|illegal_move|room_not_found|invalid
	Message string `json:"message"`
}
