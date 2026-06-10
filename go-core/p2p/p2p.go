// Package p2p implements STUN-based NAT discovery and UDP hole punching.
package p2p

import (
	"encoding/binary"
	"fmt"
	"log"
	"net"
	"time"
)

const (
	stunTimeout    = 5 * time.Second
	punchAttempts  = 10
	punchInterval  = 300 * time.Millisecond
)

// STUNDiscover queries a STUN server to find the external (public) address.
// Uses RFC 5389 Binding Request.
func STUNDiscover(stunServer string) (*net.UDPAddr, error) {
	serverAddr, err := net.ResolveUDPAddr("udp", stunServer)
	if err != nil {
		return nil, fmt.Errorf("resolve STUN server %s: %w", stunServer, err)
	}

	conn, err := net.ListenUDP("udp", nil)
	if err != nil {
		return nil, fmt.Errorf("listen UDP: %w", err)
	}
	defer conn.Close()

	// Generate transaction ID
	txID := make([]byte, 12)
	now := time.Now().UnixNano()
	for i := 0; i < 12; i++ {
		txID[i] = byte((now >> (i * 8)) & 0xFF)
	}

	// Build STUN Binding Request
	req := make([]byte, 20)
	binary.BigEndian.PutUint16(req[0:2], 0x0001) // Binding Request
	binary.BigEndian.PutUint16(req[2:4], 0x0000) // Length = 0
	binary.BigEndian.PutUint32(req[4:8], 0x2112A442) // Magic Cookie
	copy(req[8:20], txID)

	if _, err := conn.WriteToUDP(req, serverAddr); err != nil {
		return nil, fmt.Errorf("send STUN request: %w", err)
	}

	// Set deadline
	_ = conn.SetReadDeadline(time.Now().Add(stunTimeout))

	buf := make([]byte, 256)
	n, _, err := conn.ReadFromUDP(buf)
	if err != nil {
		return nil, fmt.Errorf("STUN timeout or read error: %w", err)
	}

	if n < 20 {
		return nil, fmt.Errorf("STUN response too short: %d bytes", n)
	}

	// Parse response - look for XOR-MAPPED-ADDRESS (0x0020)
	magic := uint32(0x2112A442)
	msgLen := binary.BigEndian.Uint16(buf[2:4])
	pos := 20

	for pos+4 <= 20+int(msgLen) && pos+4 <= n {
		attrType := binary.BigEndian.Uint16(buf[pos : pos+2])
		attrLen := binary.BigEndian.Uint16(buf[pos+2 : pos+4])
		pos += 4

		if attrType == 0x0020 && attrLen >= 8 {
			// XOR-MAPPED-ADDRESS
			port := binary.BigEndian.Uint16(buf[pos+2:pos+4]) ^ uint16(magic>>16)
			ip := binary.BigEndian.Uint32(buf[pos+4:pos+8]) ^ magic
			addr := &net.UDPAddr{
				IP:   net.IPv4(byte(ip>>24), byte(ip>>16), byte(ip>>8), byte(ip)),
				Port: int(port),
			}
			return addr, nil
		}

		// Align to 4 bytes
		pos += int((attrLen + 3) & ^uint16(3))
	}

	return nil, fmt.Errorf("no XOR-MAPPED-ADDRESS in STUN response")
}

// HolePunch attempts UDP hole punching to establish a direct connection.
func HolePunch(localConn *net.UDPConn, remoteAddr *net.UDPAddr) bool {
	punch := []byte("RHPR-PUNCH")

	for i := 0; i < punchAttempts; i++ {
		_, _ = localConn.WriteToUDP(punch, remoteAddr)

		_ = localConn.SetReadDeadline(time.Now().Add(punchInterval))
		buf := make([]byte, 32)
		n, addr, err := localConn.ReadFromUDP(buf)
		if err == nil && n > 0 && addr.String() == remoteAddr.String() {
			log.Printf("[p2p] hole punch successful to %s", remoteAddr)
			return true
		}
	}

	log.Printf("[p2p] hole punch failed to %s after %d attempts", remoteAddr, punchAttempts)
	return false
}

// ExchangeAndPunch performs the full P2P flow:
// 1. Discover own external address via STUN
// 2. Exchange addresses with peer (via signaling server)
// 3. Attempt hole punch
// Returns the punched UDP connection or an error.
func ExchangeAndPunch(stunServer string, signalingConn net.Conn) (*net.UDPConn, *net.UDPAddr, error) {
	// Discover our external address
	extAddr, err := STUNDiscover(stunServer)
	if err != nil {
		return nil, nil, fmt.Errorf("STUN discover: %w", err)
	}
	log.Printf("[p2p] external address: %s", extAddr)

	// Send our address to peer via signaling
	ourAddrStr := extAddr.String()
	addrBytes := []byte(ourAddrStr)
	header := make([]byte, 2)
	binary.BigEndian.PutUint16(header, uint16(len(addrBytes)))
	if _, err := signalingConn.Write(header); err != nil {
		return nil, nil, err
	}
	if _, err := signalingConn.Write(addrBytes); err != nil {
		return nil, nil, err
	}

	// Receive peer's address
	if _, err := signalingConn.Read(header); err != nil {
		return nil, nil, err
	}
	peerLen := binary.BigEndian.Uint16(header)
	peerBuf := make([]byte, peerLen)
	if _, err := signalingConn.Read(peerBuf); err != nil {
		return nil, nil, err
	}
	peerAddr, err := net.ResolveUDPAddr("udp", string(peerBuf))
	if err != nil {
		return nil, nil, fmt.Errorf("resolve peer addr: %w", err)
	}
	log.Printf("[p2p] peer address: %s", peerAddr)

	// Create local UDP socket on same port as STUN used
	localAddr := &net.UDPAddr{Port: extAddr.Port}
	localConn, err := net.ListenUDP("udp", localAddr)
	if err != nil {
		// If port is taken, try any port
		localConn, err = net.ListenUDP("udp", nil)
		if err != nil {
			return nil, nil, fmt.Errorf("listen UDP for punch: %w", err)
		}
	}

	// Attempt hole punch
	if !HolePunch(localConn, peerAddr) {
		localConn.Close()
		return nil, nil, fmt.Errorf("hole punch failed")
	}

	return localConn, peerAddr, nil
}
