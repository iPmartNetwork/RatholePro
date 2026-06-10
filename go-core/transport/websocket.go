// Package transport - WebSocket upgrade for tunneling through HTTP firewalls.
package transport

import (
	"bufio"
	"crypto/sha256"
	"encoding/base64"
	"fmt"
	"net"
	"strings"

	"github.com/iPmartNetwork/RatholePro/go-core/config"
)

// WSClientUpgrade performs a WebSocket client handshake (upgrade).
// After upgrade, the connection is used as raw TCP.
func WSClientUpgrade(conn net.Conn, host string, cfg *config.WebSocketConfig) error {
	path := "/tunnel"
	if cfg != nil && cfg.Path != "" {
		path = cfg.Path
	}

	key := "dGhlIHNhbXBsZSBub25jZQ==" // static nonce for simplicity
	req := fmt.Sprintf(
		"GET %s HTTP/1.1\r\nHost: %s\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: %s\r\nSec-WebSocket-Version: 13\r\n\r\n",
		path, host, key,
	)

	if _, err := conn.Write([]byte(req)); err != nil {
		return fmt.Errorf("ws upgrade write: %w", err)
	}

	// Read response
	reader := bufio.NewReader(conn)
	statusLine, err := reader.ReadString('\n')
	if err != nil {
		return fmt.Errorf("ws upgrade read status: %w", err)
	}
	if !strings.Contains(statusLine, "101") {
		return fmt.Errorf("ws upgrade failed: %s", strings.TrimSpace(statusLine))
	}

	// Consume remaining headers
	for {
		line, err := reader.ReadString('\n')
		if err != nil {
			return fmt.Errorf("ws upgrade read headers: %w", err)
		}
		if strings.TrimSpace(line) == "" {
			break
		}
	}

	return nil
}

// WSServerUpgrade performs a WebSocket server handshake (accept upgrade).
// After upgrade, the connection is used as raw TCP.
func WSServerUpgrade(conn net.Conn, cfg *config.WebSocketConfig) error {
	reader := bufio.NewReader(conn)

	// Read request line
	reqLine, err := reader.ReadString('\n')
	if err != nil {
		return fmt.Errorf("ws accept read: %w", err)
	}
	_ = reqLine // We don't validate path strictly

	// Read headers
	var wsKey string
	for {
		line, err := reader.ReadString('\n')
		if err != nil {
			return fmt.Errorf("ws accept read headers: %w", err)
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
		return fmt.Errorf("missing Sec-WebSocket-Key header")
	}

	// Compute accept key (SHA-256 in our implementation, matching Rust version)
	magic := wsKey + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
	hash := sha256.Sum256([]byte(magic))
	accept := base64.StdEncoding.EncodeToString(hash[:])

	resp := fmt.Sprintf(
		"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: %s\r\n\r\n",
		accept,
	)

	if _, err := conn.Write([]byte(resp)); err != nil {
		return fmt.Errorf("ws accept write: %w", err)
	}

	return nil
}
