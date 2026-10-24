package room

import (
	"sync"

	"github.com/midtone/connect-four-service/internal/protocol"
)

// Queue is a single-slot random matchmaker: the first searcher waits, and the
// next one pairs with it. Both are told (via match_found) which freshly created
// room to navigate to, where they open a normal play connection and join.
type Queue struct {
	mgr     *Manager
	mu      sync.Mutex
	waiting Player
}

func newQueue(mgr *Manager) *Queue {
	return &Queue{mgr: mgr}
}

// Find enqueues p, or pairs it with a waiting player into a new room.
func (q *Queue) Find(p Player) {
	q.mu.Lock()
	if q.waiting != nil && q.waiting != p {
		opp := q.waiting
		q.waiting = nil
		q.mu.Unlock()

		r := q.mgr.Create()
		msg := protocol.MatchFound{Type: "match_found", Code: r.Code()}
		opp.Send(msg)
		p.Send(msg)
		return
	}
	q.waiting = p
	q.mu.Unlock()
}

// Remove drops p from the queue if it is the one waiting (e.g. it disconnected
// before a match was found).
func (q *Queue) Remove(p Player) {
	q.mu.Lock()
	if q.waiting == p {
		q.waiting = nil
	}
	q.mu.Unlock()
}
