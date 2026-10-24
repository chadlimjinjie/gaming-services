package room

import (
	"crypto/rand"
	"sync"
)

// codeAlphabet excludes visually ambiguous characters (0/O, 1/I/L).
const codeAlphabet = "ABCDEFGHJKMNPQRSTUVWXYZ23456789"

const codeLen = 6

// Manager owns the live rooms and the matchmaking queue. The map is guarded by
// a mutex; per-room state lives behind each room's own goroutine.
type Manager struct {
	mu    sync.Mutex
	rooms map[string]*Room
	queue *Queue
}

func NewManager() *Manager {
	m := &Manager{rooms: make(map[string]*Room)}
	m.queue = newQueue(m)
	return m
}

// Create mints a fresh room with a unique code and starts its goroutine.
func (m *Manager) Create() *Room {
	m.mu.Lock()
	defer m.mu.Unlock()
	code := m.uniqueCodeLocked()
	r := newRoom(code, m)
	m.rooms[code] = r
	go r.run()
	return r
}

// Get returns the room for a code, if it still exists.
func (m *Manager) Get(code string) (*Room, bool) {
	m.mu.Lock()
	defer m.mu.Unlock()
	r, ok := m.rooms[code]
	return r, ok
}

// Queue exposes the matchmaking queue.
func (m *Manager) Queue() *Queue { return m.queue }

func (m *Manager) remove(code string) {
	m.mu.Lock()
	defer m.mu.Unlock()
	delete(m.rooms, code)
}

func (m *Manager) uniqueCodeLocked() string {
	for {
		code := randomCode()
		if _, exists := m.rooms[code]; !exists {
			return code
		}
	}
}

func randomCode() string {
	buf := make([]byte, codeLen)
	if _, err := rand.Read(buf); err != nil {
		// crypto/rand should never fail; fall back to a fixed pattern rather
		// than panicking a live server.
		for i := range buf {
			buf[i] = 0
		}
	}
	out := make([]byte, codeLen)
	for i, b := range buf {
		out[i] = codeAlphabet[int(b)%len(codeAlphabet)]
	}
	return string(out)
}
