// Package heartbeat implements keepalive/ping-pong over the control protocol.
package heartbeat

import (
	"log"
	"net"
	"sync"
	"time"

	"github.com/iPmartNetwork/RatholePro/go-core/protocol"
)

// ServerHeartbeat sends Ping at regular intervals and expects Pong.
// Runs until the connection is closed or timeout.
func ServerHeartbeat(conn net.Conn, interval time.Duration) {
	if interval <= 0 {
		interval = 30 * time.Second
	}

	ticker := time.NewTicker(interval)
	defer ticker.Stop()

	for range ticker.C {
		env := &protocol.Envelope{Ping: &struct{}{}}
		if err := protocol.WriteMessage(conn, env); err != nil {
			log.Printf("[heartbeat] ping write error: %v", err)
			return
		}
	}
}

// ClientHeartbeat reads Ping and responds with Pong.
// Also monitors for timeout.
func ClientHeartbeat(conn net.Conn, timeout time.Duration, stopCh <-chan struct{}) {
	if timeout <= 0 {
		timeout = 40 * time.Second
	}

	for {
		select {
		case <-stopCh:
			return
		default:
		}

		_ = conn.SetReadDeadline(time.Now().Add(timeout))
		env, err := protocol.ReadMessage(conn)
		if err != nil {
			log.Printf("[heartbeat] read error: %v", err)
			return
		}

		if env.Ping != nil {
			pong := &protocol.Envelope{Pong: &struct{}{}}
			if err := protocol.WriteMessage(conn, pong); err != nil {
				log.Printf("[heartbeat] pong write error: %v", err)
				return
			}
		}
	}
}

// Monitor watches a connection and reports when heartbeat fails.
type Monitor struct {
	conn     net.Conn
	interval time.Duration
	timeout  time.Duration
	stopCh   chan struct{}
	once     sync.Once
	Dead     chan struct{} // closed when heartbeat fails
}

// NewMonitor creates a heartbeat monitor.
func NewMonitor(conn net.Conn, interval, timeout time.Duration) *Monitor {
	if interval <= 0 {
		interval = 30 * time.Second
	}
	if timeout <= 0 {
		timeout = interval + 10*time.Second
	}
	return &Monitor{
		conn:     conn,
		interval: interval,
		timeout:  timeout,
		stopCh:   make(chan struct{}),
		Dead:     make(chan struct{}),
	}
}

// Start begins sending heartbeats.
func (m *Monitor) Start() {
	go func() {
		ticker := time.NewTicker(m.interval)
		defer ticker.Stop()
		for {
			select {
			case <-m.stopCh:
				return
			case <-ticker.C:
				env := &protocol.Envelope{Ping: &struct{}{}}
				_ = m.conn.SetWriteDeadline(time.Now().Add(5 * time.Second))
				if err := protocol.WriteMessage(m.conn, env); err != nil {
					m.once.Do(func() { close(m.Dead) })
					return
				}
			}
		}
	}()
}

// Stop halts the heartbeat monitor.
func (m *Monitor) Stop() {
	m.once.Do(func() { close(m.stopCh) })
}
