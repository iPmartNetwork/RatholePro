package client

import (
	"fmt"
	"log"
	"net"
	"sync"
	"time"

	"github.com/hashicorp/yamux"
	"github.com/iPmartNetwork/RatholePro/go-core/config"
	"github.com/iPmartNetwork/RatholePro/go-core/protocol"
	"github.com/iPmartNetwork/RatholePro/go-core/relay"
	"github.com/iPmartNetwork/RatholePro/go-core/transport"
	"github.com/iPmartNetwork/RatholePro/go-core/udp"
)

// Run starts the client with full transport support.
func Run(cfg *config.Config) error {
	cc := cfg.Client
	if cc == nil {
		return fmt.Errorf("no [client] section")
	}

	log.Printf("[client] server: %s (transport: %s)", cc.RemoteAddr, config.GetTransportType(cc.Transport))

	var wg sync.WaitGroup

	for name, svc := range cc.Services {
		muxConns := svc.MuxStreams
		if muxConns <= 0 {
			muxConns = cc.MuxConnections
		}
		if muxConns <= 0 {
			muxConns = 4
		}

		svcType := svc.Type
		if svcType == "" {
			svcType = "tcp"
		}

		for i := 0; i < muxConns; i++ {
			wg.Add(1)
			go func(name string, svc *config.ServiceConfig, idx int, svcType string) {
				defer wg.Done()
				retryInterval := time.Duration(svc.RetryInterval) * time.Second
				if retryInterval <= 0 {
					retryInterval = time.Duration(cc.RetryInterval) * time.Second
				}
				if retryInterval <= 0 {
					retryInterval = 3 * time.Second
				}

				for {
					var err error
					if svcType == "udp" {
						err = runUDPSession(name, svc, cc)
					} else {
						err = runTCPSession(name, svc, cc)
					}
					if err != nil {
						log.Printf("[%s#%d] session error: %v, retrying in %s", name, idx, err, retryInterval)
					} else {
						log.Printf("[%s#%d] session ended, reconnecting", name, idx)
					}
					time.Sleep(retryInterval)
				}
			}(name, svc, i, svcType)
		}

		log.Printf("[client] [%s] type=%s mux_connections=%d -> %s", name, svcType, muxConns, cc.RemoteAddr)
	}

	wg.Wait()
	return nil
}

// runTCPSession connects, authenticates, upgrades to yamux, and accepts streams for TCP relay.
func runTCPSession(name string, svc *config.ServiceConfig, cc *config.ClientConfig) error {
	conn, err := dialWithTransport(cc)
	if err != nil {
		return err
	}

	if err := authenticate(conn, name, svc, cc); err != nil {
		conn.Close()
		return err
	}

	log.Printf("[%s] authenticated, starting yamux session (TCP)", name)

	yamuxCfg := yamux.DefaultConfig()
	yamuxCfg.MaxStreamWindowSize = 1024 * 1024
	session, err := yamux.Client(conn, yamuxCfg)
	if err != nil {
		conn.Close()
		return fmt.Errorf("yamux client: %w", err)
	}
	defer session.Close()

	for {
		stream, err := session.Accept()
		if err != nil {
			return fmt.Errorf("yamux accept: %w", err)
		}
		go handleTCPStream(stream, svc.LocalAddr, name)
	}
}

// runUDPSession connects, authenticates, upgrades to yamux, and runs UDP forwarding.
func runUDPSession(name string, svc *config.ServiceConfig, cc *config.ClientConfig) error {
	conn, err := dialWithTransport(cc)
	if err != nil {
		return err
	}

	if err := authenticate(conn, name, svc, cc); err != nil {
		conn.Close()
		return err
	}

	log.Printf("[%s] authenticated, starting yamux session (UDP)", name)

	yamuxCfg := yamux.DefaultConfig()
	yamuxCfg.MaxStreamWindowSize = 1024 * 1024
	session, err := yamux.Client(conn, yamuxCfg)
	if err != nil {
		conn.Close()
		return fmt.Errorf("yamux client: %w", err)
	}
	defer session.Close()

	// Accept a stream for UDP forwarding
	stream, err := session.Accept()
	if err != nil {
		return fmt.Errorf("yamux accept (UDP): %w", err)
	}

	return udp.ClientUDP(svc.LocalAddr, stream)
}

// dialWithTransport establishes a connection using the configured transport.
func dialWithTransport(cc *config.ClientConfig) (net.Conn, error) {
	conn, err := transport.ClientDial(cc.RemoteAddr, cc.Transport, 10*time.Second)
	if err != nil {
		return nil, fmt.Errorf("dial %s: %w", cc.RemoteAddr, err)
	}

	// Apply transport upgrades (Noise, WebSocket)
	conn, err = applyClientTransport(conn, cc)
	if err != nil {
		return nil, err
	}

	if tc, ok := conn.(*net.TCPConn); ok {
		_ = tc.SetNoDelay(true)
	}

	return conn, nil
}

// applyClientTransport applies Noise handshake or WebSocket upgrade.
func applyClientTransport(conn net.Conn, cc *config.ClientConfig) (net.Conn, error) {
	if cc.Transport == nil {
		return conn, nil
	}
	switch cc.Transport.Type {
	case "noise":
		return transport.NoiseClientUpgrade(conn, cc.Transport.Noise)
	case "ws":
		host := cc.RemoteAddr
		if err := transport.WSClientUpgrade(conn, host, cc.Transport.WebSocket); err != nil {
			conn.Close()
			return nil, err
		}
		return conn, nil
	}
	return conn, nil
}

// authenticate sends auth and validates response.
func authenticate(conn net.Conn, name string, svc *config.ServiceConfig, cc *config.ClientConfig) error {
	token := config.GetServiceToken(svc.Token, cc.DefaultToken)
	if token == "" {
		return fmt.Errorf("no token for service '%s'", name)
	}

	svcType := svc.Type
	if svcType == "" {
		svcType = "tcp"
	}

	authEnv := &protocol.Envelope{
		Auth: &protocol.AuthRequest{
			ServiceName: name,
			TokenHash:   protocol.HashToken(token),
			ServiceType: svcType,
			MuxEnabled:  true,
			MuxStreams:   uint32(svc.MuxStreams),
		},
	}
	if err := protocol.WriteMessage(conn, authEnv); err != nil {
		return fmt.Errorf("write auth: %w", err)
	}

	respEnv, err := protocol.ReadMessage(conn)
	if err != nil {
		return fmt.Errorf("read auth resp: %w", err)
	}
	if respEnv.AuthResp == nil {
		return fmt.Errorf("expected AuthResp")
	}
	if !respEnv.AuthResp.Success {
		return fmt.Errorf("auth failed: %s", respEnv.AuthResp.Message)
	}

	return nil
}

// handleTCPStream connects to local service and relays.
func handleTCPStream(stream net.Conn, localAddr string, serviceName string) {
	if localAddr == "" {
		log.Printf("[%s] no local_addr configured", serviceName)
		stream.Close()
		return
	}

	local, err := net.DialTimeout("tcp", localAddr, 5*time.Second)
	if err != nil {
		log.Printf("[%s] connect local %s: %v", serviceName, localAddr, err)
		stream.Close()
		return
	}

	if tc, ok := local.(*net.TCPConn); ok {
		_ = tc.SetNoDelay(true)
	}

	log.Printf("[%s] relay: yamux stream <-> %s", serviceName, localAddr)
	relay.Relay(stream, local)
}
