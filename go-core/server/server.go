package server

import (
	"fmt"
	"log"
	"net"
	"sync"

	"github.com/hashicorp/yamux"
	"github.com/iPmartNetwork/RatholePro/go-core/config"
	"github.com/iPmartNetwork/RatholePro/go-core/protocol"
	"github.com/iPmartNetwork/RatholePro/go-core/relay"
	"github.com/iPmartNetwork/RatholePro/go-core/transport"
	"github.com/iPmartNetwork/RatholePro/go-core/udp"
)

// sessionPool holds yamux sessions for a service, ready to open streams for visitors.
type sessionPool struct {
	mu       sync.Mutex
	sessions []*yamux.Session
}

func (p *sessionPool) add(s *yamux.Session) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.sessions = append(p.sessions, s)
}

// pick returns a healthy session (round-robin style, removes dead ones).
func (p *sessionPool) pick() *yamux.Session {
	p.mu.Lock()
	defer p.mu.Unlock()

	for len(p.sessions) > 0 {
		s := p.sessions[0]
		p.sessions = p.sessions[1:]
		if !s.IsClosed() {
			p.sessions = append(p.sessions, s)
			return s
		}
	}
	return nil
}

func (p *sessionPool) remove(target *yamux.Session) {
	p.mu.Lock()
	defer p.mu.Unlock()
	for i, s := range p.sessions {
		if s == target {
			p.sessions = append(p.sessions[:i], p.sessions[i+1:]...)
			return
		}
	}
}

// udpPool holds yamux sessions dedicated to UDP services.
type udpPool struct {
	mu       sync.Mutex
	sessions []*yamux.Session
}

func (p *udpPool) add(s *yamux.Session) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.sessions = append(p.sessions, s)
}

func (p *udpPool) pick() *yamux.Session {
	p.mu.Lock()
	defer p.mu.Unlock()
	for len(p.sessions) > 0 {
		s := p.sessions[0]
		p.sessions = p.sessions[1:]
		if !s.IsClosed() {
			p.sessions = append(p.sessions, s)
			return s
		}
	}
	return nil
}

// Run starts the server with full transport support (TCP/TLS/Noise/WS).
func Run(cfg *config.Config) error {
	sc := cfg.Server
	if sc == nil {
		return fmt.Errorf("no [server] section")
	}

	// Create listener based on transport config
	controlLn, err := transport.ServerListen(sc.BindAddr, sc.Transport)
	if err != nil {
		return fmt.Errorf("listen control %s: %w", sc.BindAddr, err)
	}
	log.Printf("[server] control on %s (transport: %s)", sc.BindAddr, config.GetTransportType(sc.Transport))

	// Create session pools per service
	tcpPools := make(map[string]*sessionPool)
	udpPools := make(map[string]*udpPool)
	for name, svc := range sc.Services {
		svcType := svc.Type
		if svcType == "" {
			svcType = "tcp"
		}
		if svcType == "udp" {
			udpPools[name] = &udpPool{}
		} else {
			tcpPools[name] = &sessionPool{}
		}
	}

	// Start visitor listeners for TCP services
	for name, svc := range sc.Services {
		if svc.BindAddr == "" {
			continue
		}
		svcType := svc.Type
		if svcType == "" {
			svcType = "tcp"
		}

		if svcType == "udp" {
			// For UDP: when a client yamux session comes in, open a stream
			// and run UDP server on it
			pool := udpPools[name]
			bindAddr := svc.BindAddr
			svcName := name
			go func() {
				// Wait for at least one yamux session, then start UDP
				// We retry periodically until a session is available
				for {
					sess := pool.pick()
					if sess == nil {
						// Wait and retry
						<-make(chan struct{}) // block forever if no session
						continue
					}
					stream, err := sess.Open()
					if err != nil {
						log.Printf("[%s] yamux open for UDP: %v", svcName, err)
						continue
					}
					log.Printf("[%s] UDP server starting on %s", svcName, bindAddr)
					if err := udp.ServerUDP(bindAddr, stream); err != nil {
						log.Printf("[%s] UDP server error: %v", svcName, err)
					}
				}
			}()
		} else {
			// TCP service visitor listener
			ln, err := net.Listen("tcp", svc.BindAddr)
			if err != nil {
				return fmt.Errorf("[%s] listen visitor %s: %w", name, svc.BindAddr, err)
			}
			log.Printf("[server] [%s] visitors on %s", name, svc.BindAddr)
			go acceptVisitors(ln, tcpPools[name], name)
		}
	}

	// Accept client connections
	for {
		conn, err := controlLn.Accept()
		if err != nil {
			log.Printf("[server] accept error: %v", err)
			continue
		}

		// Apply transport upgrades
		conn, err = applyServerTransport(conn, sc.Transport)
		if err != nil {
			log.Printf("[server] transport upgrade: %v", err)
			continue
		}

		go handleClientConn(conn, sc, tcpPools, udpPools)
	}
}

// applyServerTransport applies Noise or WebSocket upgrades on accepted connections.
func applyServerTransport(conn net.Conn, t *config.TransportConfig) (net.Conn, error) {
	if t == nil {
		return conn, nil
	}
	switch t.Type {
	case "noise":
		return transport.NoiseServerUpgrade(conn, t.Noise)
	case "ws":
		upgraded, err := transport.WSServerUpgrade(conn, t.WebSocket)
		if err != nil {
			conn.Close()
			return nil, err
		}
		return upgraded, nil
	}
	return conn, nil
}

// handleClientConn authenticates and creates a yamux session.
func handleClientConn(conn net.Conn, sc *config.ServerConfig, tcpPools map[string]*sessionPool, udpPools map[string]*udpPool) {
	defer func() {
		if r := recover(); r != nil {
			log.Printf("[server] panic: %v", r)
		}
	}()

	env, err := protocol.ReadMessage(conn)
	if err != nil {
		log.Printf("[server] read auth: %v", err)
		conn.Close()
		return
	}
	if env.Auth == nil {
		log.Printf("[server] expected Auth message")
		conn.Close()
		return
	}

	auth := env.Auth
	svc, ok := sc.Services[auth.ServiceName]
	if !ok {
		sendAuthFail(conn, fmt.Sprintf("unknown service: %s", auth.ServiceName))
		return
	}

	token := config.GetServiceToken(svc.Token, sc.DefaultToken)
	if token == "" {
		sendAuthFail(conn, "no token configured")
		return
	}
	if auth.TokenHash != protocol.HashToken(token) {
		sendAuthFail(conn, "bad token")
		return
	}

	// Auth OK
	resp := &protocol.Envelope{
		AuthResp: &protocol.AuthResponse{
			Success: true,
			Message: "OK",
		},
	}
	if err := protocol.WriteMessage(conn, resp); err != nil {
		conn.Close()
		return
	}

	svcType := svc.Type
	if svcType == "" {
		svcType = "tcp"
	}

	log.Printf("[server] client authenticated for '%s' (type=%s, yamux)", auth.ServiceName, svcType)

	// Upgrade to yamux session
	yamuxCfg := yamux.DefaultConfig()
	yamuxCfg.MaxStreamWindowSize = 1024 * 1024
	session, err := yamux.Server(conn, yamuxCfg)
	if err != nil {
		log.Printf("[server] yamux error: %v", err)
		conn.Close()
		return
	}

	if svcType == "udp" {
		pool := udpPools[auth.ServiceName]
		if pool == nil {
			log.Printf("[server] no UDP pool for '%s'", auth.ServiceName)
			session.Close()
			return
		}
		pool.add(session)
		log.Printf("[server] yamux session (UDP) added for '%s'", auth.ServiceName)
	} else {
		pool := tcpPools[auth.ServiceName]
		if pool == nil {
			log.Printf("[server] no TCP pool for '%s'", auth.ServiceName)
			session.Close()
			return
		}
		pool.add(session)
		log.Printf("[server] yamux session added for '%s'", auth.ServiceName)
	}

	<-session.CloseChan()
	log.Printf("[server] yamux session closed for '%s'", auth.ServiceName)
}

// acceptVisitors handles incoming TCP visitor connections.
func acceptVisitors(ln net.Listener, pool *sessionPool, serviceName string) {
	for {
		visitor, err := ln.Accept()
		if err != nil {
			log.Printf("[%s] visitor accept error: %v", serviceName, err)
			return
		}
		go handleVisitor(visitor, pool, serviceName)
	}
}

// handleVisitor opens a yamux stream and relays.
func handleVisitor(visitor net.Conn, pool *sessionPool, serviceName string) {
	session := pool.pick()
	if session == nil {
		log.Printf("[%s] no client available for visitor %s", serviceName, visitor.RemoteAddr())
		visitor.Close()
		return
	}

	stream, err := session.Open()
	if err != nil {
		log.Printf("[%s] yamux open stream error: %v", serviceName, err)
		visitor.Close()
		if session.IsClosed() {
			pool.remove(session)
		}
		return
	}

	log.Printf("[%s] relay: visitor %s <-> yamux stream", serviceName, visitor.RemoteAddr())
	relay.Relay(visitor, stream)
}

func sendAuthFail(conn net.Conn, msg string) {
	resp := &protocol.Envelope{
		AuthResp: &protocol.AuthResponse{
			Success: false,
			Message: msg,
		},
	}
	_ = protocol.WriteMessage(conn, resp)
	conn.Close()
}
