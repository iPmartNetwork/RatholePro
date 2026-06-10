package protocol

import (
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
)

const (
	ProtocolVersion = 1
	MaxFrameSize    = 16 * 1024 * 1024 // 16 MB
)

var Magic = [4]byte{'R', 'H', 'P', 'R'}

// Message types for control protocol (auth/heartbeat).
type MessageType string

const (
	MsgAuth       MessageType = "Auth"
	MsgAuthResp   MessageType = "AuthResp"
	MsgPing       MessageType = "Ping"
	MsgPong       MessageType = "Pong"
	MsgDisconnect MessageType = "Disconnect"
)

// Envelope wraps different message types for JSON serialization,
// compatible with the Rust serde enum representation.
type Envelope struct {
	Auth       *AuthRequest  `json:"Auth,omitempty"`
	AuthResp   *AuthResponse `json:"AuthResp,omitempty"`
	Ping       *struct{}     `json:"Ping,omitempty"`
	Pong       *struct{}     `json:"Pong,omitempty"`
	Disconnect *string       `json:"Disconnect,omitempty"`
}

type AuthRequest struct {
	ServiceName string `json:"service_name"`
	TokenHash   string `json:"token_hash"`
	ServiceType string `json:"service_type"`
	MuxEnabled  bool   `json:"mux_enabled"`
	MuxStreams  uint32 `json:"mux_streams"`
}

type AuthResponse struct {
	Success   bool   `json:"success"`
	Message   string `json:"message"`
	SessionID string `json:"session_id,omitempty"`
}

// HashToken produces SHA-256 hex of the token, matching the Rust implementation.
func HashToken(token string) string {
	h := sha256.Sum256([]byte(token))
	return hex.EncodeToString(h[:])
}

// WriteMessage writes a framed message (MAGIC + version + len + JSON payload).
func WriteMessage(w io.Writer, env *Envelope) error {
	payload, err := json.Marshal(env)
	if err != nil {
		return fmt.Errorf("marshal message: %w", err)
	}
	if len(payload) > MaxFrameSize {
		return fmt.Errorf("message too large: %d bytes", len(payload))
	}

	// Header: 4 magic + 1 version + 4 length = 9 bytes
	header := make([]byte, 9)
	copy(header[0:4], Magic[:])
	header[4] = ProtocolVersion
	binary.BigEndian.PutUint32(header[5:9], uint32(len(payload)))

	if _, err := w.Write(header); err != nil {
		return err
	}
	_, err = w.Write(payload)
	return err
}

// ReadMessage reads a framed message from the reader.
func ReadMessage(r io.Reader) (*Envelope, error) {
	header := make([]byte, 9)
	if _, err := io.ReadFull(r, header); err != nil {
		return nil, fmt.Errorf("read header: %w", err)
	}

	if header[0] != Magic[0] || header[1] != Magic[1] ||
		header[2] != Magic[2] || header[3] != Magic[3] {
		return nil, fmt.Errorf("invalid magic bytes")
	}
	if header[4] != ProtocolVersion {
		return nil, fmt.Errorf("unsupported protocol version: %d", header[4])
	}

	length := binary.BigEndian.Uint32(header[5:9])
	if length > MaxFrameSize {
		return nil, fmt.Errorf("frame too large: %d bytes", length)
	}

	payload := make([]byte, length)
	if _, err := io.ReadFull(r, payload); err != nil {
		return nil, fmt.Errorf("read payload: %w", err)
	}

	var env Envelope
	if err := json.Unmarshal(payload, &env); err != nil {
		return nil, fmt.Errorf("unmarshal message: %w", err)
	}
	return &env, nil
}
