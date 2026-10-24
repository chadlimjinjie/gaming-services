// Package ws bridges WebSocket connections to rooms. Each connection becomes a
// Client that implements room.Player; its read pump dispatches inbound protocol
// messages to the room, and its write pump drains queued outbound messages.
package ws

import (
	"encoding/json"
	"log"
	"net/http"
	"sync"
	"time"

	"github.com/gorilla/websocket"

	"github.com/midtone/connect-four-service/internal/protocol"
	"github.com/midtone/connect-four-service/internal/room"
)

const (
	writeWait      = 10 * time.Second
	pongWait       = 60 * time.Second
	pingPeriod     = 30 * time.Second
	maxMessageSize = 4 * 1024
	sendBuffer     = 32
)

// Handler upgrades HTTP requests to WebSocket game connections.
type Handler struct {
	mgr      *room.Manager
	upgrader websocket.Upgrader
}

// NewHandler builds a handler that accepts WebSocket upgrades from any origin.
// The endpoint carries no ambient (cookie/session) credentials — identity is an
// explicit ?pid= query param — so cross-site WS hijacking is not a
// concern and origin checking is intentionally disabled.
func NewHandler(mgr *room.Manager) *Handler {
	return &Handler{
		mgr: mgr,
		upgrader: websocket.Upgrader{
			ReadBufferSize:  1024,
			WriteBufferSize: 1024,
			CheckOrigin:     func(r *http.Request) bool { return true },
		},
	}
}

func (h *Handler) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	// Identity is resolved once, at the connection level: the ephemeral ?pid=.
	pid := r.URL.Query().Get("pid")
	if pid == "" {
		http.Error(w, "missing player identity", http.StatusBadRequest)
		return
	}
	identity := protocol.Identity{Kind: "anon", ID: pid}

	conn, err := h.upgrader.Upgrade(w, r, nil)
	if err != nil {
		return // upgrader already wrote the error
	}

	c := &Client{
		conn:     conn,
		identity: identity,
		mgr:      h.mgr,
		send:     make(chan []byte, sendBuffer),
		closed:   make(chan struct{}),
	}
	go c.writePump()
	go c.readPump()
}

// Client is one WebSocket connection. It satisfies room.Player.
type Client struct {
	conn     *websocket.Conn
	identity protocol.Identity
	mgr      *room.Manager
	room     *room.Room // set after a successful join; touched only by readPump

	send      chan []byte
	closed    chan struct{}
	closeOnce sync.Once
}

func (c *Client) Identity() protocol.Identity { return c.identity }

// Send marshals v and queues it for delivery. It never blocks the caller (the
// room goroutine): a client whose buffer is full is dropped.
func (c *Client) Send(v any) {
	b, err := json.Marshal(v)
	if err != nil {
		return
	}
	select {
	case <-c.closed:
		return
	default:
	}
	select {
	case c.send <- b:
	case <-c.closed:
	default:
		log.Printf("connect-four: dropping message to slow client %s", c.identity.ID)
		c.Close()
	}
}

// Close signals both pumps to shut down. Safe to call repeatedly.
func (c *Client) Close() {
	c.closeOnce.Do(func() { close(c.closed) })
}

func (c *Client) readPump() {
	defer func() {
		if c.room != nil {
			c.room.Leave(c)
		} else {
			c.mgr.Queue().Remove(c)
		}
		c.Close()
		c.conn.Close()
	}()

	c.conn.SetReadLimit(maxMessageSize)
	_ = c.conn.SetReadDeadline(time.Now().Add(pongWait))
	c.conn.SetPongHandler(func(string) error {
		return c.conn.SetReadDeadline(time.Now().Add(pongWait))
	})

	for {
		_, data, err := c.conn.ReadMessage()
		if err != nil {
			return
		}
		var in protocol.Inbound
		if err := json.Unmarshal(data, &in); err != nil {
			continue
		}
		if !c.handle(in) {
			return
		}
	}
}

// handle dispatches one inbound message. It returns false to end the read pump.
func (c *Client) handle(in protocol.Inbound) bool {
	switch in.Type {
	case "create":
		r := c.mgr.Create()
		c.room = r
		r.Join(c, sanitizeName(in.Name))
	case "join":
		r, ok := c.mgr.Get(in.Code)
		if !ok {
			c.Send(protocol.Error{Type: "error", Code: "room_not_found", Message: "That game does not exist."})
			return true
		}
		c.room = r
		r.Join(c, sanitizeName(in.Name))
	case "find_match":
		c.mgr.Queue().Find(c)
	case "move":
		if c.room != nil {
			c.room.Move(c, in.Column)
		}
	case "rematch":
		if c.room != nil {
			c.room.Rematch(c)
		}
	case "ping":
		c.Send(protocol.Simple{Type: "pong"})
	case "leave":
		return false
	}
	return true
}

func (c *Client) writePump() {
	ticker := time.NewTicker(pingPeriod)
	defer func() {
		ticker.Stop()
		c.conn.Close()
	}()
	for {
		select {
		case msg := <-c.send:
			_ = c.conn.SetWriteDeadline(time.Now().Add(writeWait))
			if err := c.conn.WriteMessage(websocket.TextMessage, msg); err != nil {
				return
			}
		case <-c.closed:
			_ = c.conn.SetWriteDeadline(time.Now().Add(writeWait))
			_ = c.conn.WriteMessage(websocket.CloseMessage,
				websocket.FormatCloseMessage(websocket.CloseNormalClosure, ""))
			return
		case <-ticker.C:
			_ = c.conn.SetWriteDeadline(time.Now().Add(writeWait))
			if err := c.conn.WriteMessage(websocket.PingMessage, nil); err != nil {
				return
			}
		}
	}
}

func sanitizeName(name string) string {
	name = trimSpace(name)
	if name == "" {
		return "Player"
	}
	if len(name) > 24 {
		name = name[:24]
	}
	return name
}

// trimSpace avoids importing strings for a single use and keeps control
// characters out of names.
func trimSpace(s string) string {
	start, end := 0, len(s)
	for start < end && s[start] <= ' ' {
		start++
	}
	for end > start && s[end-1] <= ' ' {
		end--
	}
	return s[start:end]
}
