// Package loadbalancer provides connection distribution across multiple backends.
// Strategies: round_robin, random, least_conn.
package loadbalancer

import (
	"fmt"
	"math/rand"
	"net"
	"sync"
	"sync/atomic"
	"time"
)

type Strategy int

const (
	RoundRobin Strategy = iota
	Random
	LeastConn
)

func ParseStrategy(s string) Strategy {
	switch s {
	case "random":
		return Random
	case "least_conn":
		return LeastConn
	default:
		return RoundRobin
	}
}

// LoadBalancer distributes connections across backends.
type LoadBalancer struct {
	backends    []string
	strategy    Strategy
	index       uint64
	connections []int64
	mu          sync.Mutex
	rng         *rand.Rand
}

// New creates a new LoadBalancer.
func New(backends []string, strategy Strategy) *LoadBalancer {
	conns := make([]int64, len(backends))
	return &LoadBalancer{
		backends:    backends,
		strategy:    strategy,
		connections: conns,
		rng:         rand.New(rand.NewSource(time.Now().UnixNano())),
	}
}

// Next returns the address of the next backend to use.
func (lb *LoadBalancer) Next() (string, int, error) {
	if len(lb.backends) == 0 {
		return "", -1, fmt.Errorf("no backends available")
	}

	var idx int
	switch lb.strategy {
	case RoundRobin:
		idx = int(atomic.AddUint64(&lb.index, 1)-1) % len(lb.backends)
	case Random:
		lb.mu.Lock()
		idx = lb.rng.Intn(len(lb.backends))
		lb.mu.Unlock()
	case LeastConn:
		idx = 0
		minConn := atomic.LoadInt64(&lb.connections[0])
		for i := 1; i < len(lb.backends); i++ {
			c := atomic.LoadInt64(&lb.connections[i])
			if c < minConn {
				minConn = c
				idx = i
			}
		}
	}

	return lb.backends[idx], idx, nil
}

// Connect dials the next backend and tracks the connection count.
func (lb *LoadBalancer) Connect(timeout time.Duration) (net.Conn, int, error) {
	addr, idx, err := lb.Next()
	if err != nil {
		return nil, -1, err
	}

	atomic.AddInt64(&lb.connections[idx], 1)

	conn, err := net.DialTimeout("tcp", addr, timeout)
	if err != nil {
		atomic.AddInt64(&lb.connections[idx], -1)
		return nil, -1, fmt.Errorf("backend '%s' failed: %w", addr, err)
	}

	if tc, ok := conn.(*net.TCPConn); ok {
		_ = tc.SetNoDelay(true)
	}

	return conn, idx, nil
}

// Release decrements the connection count for a backend.
func (lb *LoadBalancer) Release(idx int) {
	if idx >= 0 && idx < len(lb.connections) {
		atomic.AddInt64(&lb.connections[idx], -1)
	}
}

// Backends returns the list of backend addresses.
func (lb *LoadBalancer) Backends() []string {
	return lb.backends
}
