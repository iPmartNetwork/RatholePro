// Package transport - WebSocket upgrade + RFC 6455 framing for CDN compatibility.
package transport

import (
	"bufio"
	"crypto/rand"
	"crypto/sha1"
	"encoding/base64"
	"fmt"
	"net"
	"strings"

	"github.com/iPmartNetwork/RatholePro/go-core/config"
)

// WSClientUpgrade performs WebSocket client handshake and returns a framed WSConn.
func WSClientUpgrade(conn net.Conn, host string, cfg *config.WebSocketConfig) (net.Conn, error) {
	path := "/tunnel"
	if cfg != nil && cfg.Path != "" {
		path = cfg.Path
	}

	// Generate random key
	keyBytes := make([]byte, 16)
	_, _ = rand.Read(keyBytes)
	key := base64.StdEncoding.EncodeToString(keyBytes)

	req := fmt.Sprintf(
		"GET %s HTTP/1.1\r\nHost: %s\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: %s\r\nSec-WebSocket-Version: 13\r\n\r\n",
		path, host, key,
	)

	if _, err := conn.Write([]byte(req)); err != nil {
		return nil, fmt.Errorf("ws upgrade write: %w", err)
	}

	// Read response
	reader := bufio.NewReader(conn)
	statusLine, err := reader.ReadString('\n')
	if err != nil {
		return nil, fmt.Errorf("ws upgrade read status: %w", err)
	}
	if !strings.Contains(statusLine, "101") {
		return nil, fmt.Errorf("ws upgrade failed: %s", strings.TrimSpace(statusLine))
	}

	// Consume remaining headers
	for {
		line, err := reader.ReadString('\n')
		if err != nil {
			return nil, fmt.Errorf("ws upgrade read headers: %w", err)
		}
		if strings.TrimSpace(line) == "" {
			break
		}
	}

	// Return WSConn with proper framing (client masks frames)
	return NewWSConn(conn, true), nil
}

// WSServerUpgrade performs WebSocket server handshake and returns a framed WSConn.
func WSServerUpgrade(conn net.Conn, cfg *config.WebSocketConfig) (net.Conn, error) {
	reader := bufio.NewReader(conn)

	// Read request line
	reqLine, err := reader.ReadString('\n')
	if err != nil {
		return nil, fmt.Errorf("ws accept read: %w", err)
	}
	_ = reqLine

	// Read headers
	var wsKey string
	for {
		line, err := reader.ReadString('\n')
		if err != nil {
			return nil, fmt.Errorf("ws accept read headers: %w", err)
		}
		trimmed := strings.TrimSpace(line)
		if trimmed == "" {
			break
		}
		lower := strings.ToLower(trimmed)
		if strings.HasPrefix(lower, "sec-websocket-key") {
			parts := strings.SplitN(trimmed, ":", 2)
			if len(parts) == 2 {
				wsKey = strings.TrimSpace(parts[1])
			}
		}
	}

	if wsKey == "" {
		return nil, fmt.Errorf("missing Sec-WebSocket-Key header")
	}

	// Compute accept key per RFC 6455 (SHA-1, not SHA-256!)
	magic := wsKey + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
	hash := sha1.Sum([]byte(magic))
	accept := base64.StdEncoding.EncodeToString(hash[:])

	resp := fmt.Sprintf(
		"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: %s\r\n\r\n",
		accept,
	)

	if _, err := conn.Write([]byte(resp)); err != nil {
		return nil, fmt.Errorf("ws accept write: %w", err)
	}

	// Return WSConn with proper framing (server does NOT mask)
	return NewWSConn(conn, false), nil
}
