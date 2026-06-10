// Package udp implements UDP forwarding over yamux streams.
// UDP packets are encapsulated in length-prefixed frames over a reliable stream.
package udp

import (
	"encoding/binary"
	"io"
	"log"
	"net"
	"sync"
	"time"
)

const (
	sessionTimeout = 60 * time.Second
	cleanupTick    = 30 * time.Second
	maxUDPPacket   = 65535
)

// udpSession tracks a UDP client session.
type udpSession struct {
	addr     *net.UDPAddr
	lastSeen time.Time
}

// ServerUDP listens on a UDP port and forwards packets through the yamux stream.
// Each UDP "session" (by source addr) maps to the same stream.
// Frame format on stream: [2 bytes big-endian length][payload]
func ServerUDP(bindAddr string, stream net.Conn) error {
	udpAddr, err := net.ResolveUDPAddr("udp", bindAddr)
	if err != nil {
		return err
	}
	sock, err := net.ListenUDP("udp", udpAddr)
	if err != nil {
		return err
	}
	defer sock.Close()
	log.Printf("[udp-server] listening on %s", bindAddr)

	var mu sync.Mutex
	sessions := make(map[string]*udpSession)

	// Cleanup expired sessions
	go func() {
		ticker := time.NewTicker(cleanupTick)
		defer ticker.Stop()
		for range ticker.C {
			mu.Lock()
			now := time.Now()
			for k, s := range sessions {
				if now.Sub(s.lastSeen) > sessionTimeout {
					delete(sessions, k)
				}
			}
			mu.Unlock()
		}
	}()

	// Read from UDP socket -> write to stream
	go func() {
		buf := make([]byte, maxUDPPacket)
		for {
			n, addr, err := sock.ReadFromUDP(buf)
			if err != nil {
				log.Printf("[udp-server] read error: %v", err)
				return
			}

			mu.Lock()
			key := addr.String()
			sessions[key] = &udpSession{addr: addr, lastSeen: time.Now()}
			mu.Unlock()

			// Write frame: [2-byte addr-key-len][addr-key][2-byte payload-len][payload]
			addrBytes := []byte(key)
			frame := make([]byte, 2+len(addrBytes)+2+n)
			binary.BigEndian.PutUint16(frame[0:2], uint16(len(addrBytes)))
			copy(frame[2:2+len(addrBytes)], addrBytes)
			binary.BigEndian.PutUint16(frame[2+len(addrBytes):4+len(addrBytes)], uint16(n))
			copy(frame[4+len(addrBytes):], buf[:n])

			if _, err := stream.Write(frame); err != nil {
				log.Printf("[udp-server] stream write error: %v", err)
				return
			}
		}
	}()

	// Read from stream -> write to UDP socket
	for {
		// Read addr key length
		header := make([]byte, 2)
		if _, err := io.ReadFull(stream, header); err != nil {
			return err
		}
		addrKeyLen := binary.BigEndian.Uint16(header)
		addrKeyBuf := make([]byte, addrKeyLen)
		if _, err := io.ReadFull(stream, addrKeyBuf); err != nil {
			return err
		}

		// Read payload length
		if _, err := io.ReadFull(stream, header); err != nil {
			return err
		}
		payloadLen := binary.BigEndian.Uint16(header)
		payload := make([]byte, payloadLen)
		if _, err := io.ReadFull(stream, payload); err != nil {
			return err
		}

		addrKey := string(addrKeyBuf)
		mu.Lock()
		sess, ok := sessions[addrKey]
		mu.Unlock()
		if ok {
			_, _ = sock.WriteToUDP(payload, sess.addr)
		}
	}
}

// ClientUDP connects to a local UDP service and relays packets via the yamux stream.
// Frame format on stream: [2 bytes big-endian length][payload] (simplified, no addr needed)
func ClientUDP(localAddr string, stream net.Conn) error {
	targetAddr, err := net.ResolveUDPAddr("udp", localAddr)
	if err != nil {
		return err
	}

	// Bind an ephemeral UDP socket and "connect" to local target
	localUDP, err := net.DialUDP("udp", nil, targetAddr)
	if err != nil {
		return err
	}
	defer localUDP.Close()
	log.Printf("[udp-client] forwarding to %s", localAddr)

	// Read from stream -> send to local UDP
	go func() {
		for {
			// Read frame: [2-byte addr-key-len][addr-key][2-byte payload-len][payload]
			header := make([]byte, 2)
			if _, err := io.ReadFull(stream, header); err != nil {
				return
			}
			addrKeyLen := binary.BigEndian.Uint16(header)
			// Skip addr key (client doesn't need it)
			if addrKeyLen > 0 {
				skip := make([]byte, addrKeyLen)
				if _, err := io.ReadFull(stream, skip); err != nil {
					return
				}
			}

			if _, err := io.ReadFull(stream, header); err != nil {
				return
			}
			payloadLen := binary.BigEndian.Uint16(header)
			payload := make([]byte, payloadLen)
			if _, err := io.ReadFull(stream, payload); err != nil {
				return
			}

			_, _ = localUDP.Write(payload)
		}
	}()

	// Read from local UDP -> write to stream
	buf := make([]byte, maxUDPPacket)
	emptyAddr := []byte{}
	for {
		n, err := localUDP.Read(buf)
		if err != nil {
			return err
		}

		// Frame: [0-byte addr][2-byte payload-len][payload]
		frame := make([]byte, 2+0+2+n)
		binary.BigEndian.PutUint16(frame[0:2], uint16(len(emptyAddr)))
		binary.BigEndian.PutUint16(frame[2:4], uint16(n))
		copy(frame[4:], buf[:n])

		if _, err := stream.Write(frame); err != nil {
			return err
		}
	}
}
