// Package transport - RFC 6455 WebSocket framing.
// Wraps a net.Conn to send/receive data inside proper WebSocket binary frames.
// This is required for CDN compatibility (Arvan, Cloudflare, etc.)
package transport

import (
	"crypto/rand"
	"encoding/binary"
	"fmt"
	"io"
	"net"
	"sync"
	"time"
)

// WSConn wraps a net.Conn with WebSocket binary framing (RFC 6455).
// After the HTTP upgrade, all data goes through WebSocket frames.
type WSConn struct {
	raw      net.Conn
	isClient bool // client must mask frames
	readBuf  []byte
	readPos  int
	writeMu  sync.Mutex
}

// NewWSConn wraps a connection with WebSocket framing.
func NewWSConn(raw net.Conn, isClient bool) *WSConn {
	return &WSConn{
		raw:      raw,
		isClient: isClient,
	}
}

func (ws *WSConn) Read(b []byte) (int, error) {
	// If we have buffered data from a previous frame, return it
	if ws.readPos < len(ws.readBuf) {
		n := copy(b, ws.readBuf[ws.readPos:])
		ws.readPos += n
		if ws.readPos >= len(ws.readBuf) {
			ws.readBuf = nil
			ws.readPos = 0
		}
		return n, nil
	}

	// Read a new WebSocket frame
	payload, err := ws.readFrame()
	if err != nil {
		return 0, err
	}

	n := copy(b, payload)
	if n < len(payload) {
		ws.readBuf = payload
		ws.readPos = n
	}
	return n, nil
}

func (ws *WSConn) Write(b []byte) (int, error) {
	ws.writeMu.Lock()
	defer ws.writeMu.Unlock()

	// Write as binary frame (opcode 0x02)
	err := ws.writeFrame(0x02, b)
	if err != nil {
		return 0, err
	}
	return len(b), nil
}

// readFrame reads one WebSocket frame and returns the payload.
func (ws *WSConn) readFrame() ([]byte, error) {
	// Read first 2 bytes
	header := make([]byte, 2)
	if _, err := io.ReadFull(ws.raw, header); err != nil {
		return nil, fmt.Errorf("ws read header: %w", err)
	}

	// FIN bit + opcode
	opcode := header[0] & 0x0F
	masked := (header[1] & 0x80) != 0
	payloadLen := uint64(header[1] & 0x7F)

	// Handle close frame
	if opcode == 0x08 {
		return nil, io.EOF
	}

	// Handle ping - respond with pong
	if opcode == 0x09 {
		pingData := make([]byte, payloadLen)
		if payloadLen > 0 {
			io.ReadFull(ws.raw, pingData)
		}
		ws.writeMu.Lock()
		ws.writeFrame(0x0A, pingData) // pong
		ws.writeMu.Unlock()
		return ws.readFrame() // read next real frame
	}

	// Extended payload length
	if payloadLen == 126 {
		ext := make([]byte, 2)
		if _, err := io.ReadFull(ws.raw, ext); err != nil {
			return nil, err
		}
		payloadLen = uint64(binary.BigEndian.Uint16(ext))
	} else if payloadLen == 127 {
		ext := make([]byte, 8)
		if _, err := io.ReadFull(ws.raw, ext); err != nil {
			return nil, err
		}
		payloadLen = binary.BigEndian.Uint64(ext)
	}

	// Mask key (4 bytes, only if masked)
	var maskKey [4]byte
	if masked {
		if _, err := io.ReadFull(ws.raw, maskKey[:]); err != nil {
			return nil, err
		}
	}

	// Read payload
	if payloadLen > 16*1024*1024 {
		return nil, fmt.Errorf("ws frame too large: %d", payloadLen)
	}
	payload := make([]byte, payloadLen)
	if payloadLen > 0 {
		if _, err := io.ReadFull(ws.raw, payload); err != nil {
			return nil, err
		}
	}

	// Unmask if needed
	if masked {
		for i := range payload {
			payload[i] ^= maskKey[i%4]
		}
	}

	return payload, nil
}

// writeFrame writes a WebSocket frame.
func (ws *WSConn) writeFrame(opcode byte, payload []byte) error {
	length := len(payload)

	// Calculate header size
	headerSize := 2
	if length >= 126 && length < 65536 {
		headerSize += 2
	} else if length >= 65536 {
		headerSize += 8
	}
	if ws.isClient {
		headerSize += 4 // mask key
	}

	frame := make([]byte, headerSize+length)
	pos := 0

	// Byte 0: FIN + opcode
	frame[pos] = 0x80 | opcode
	pos++

	// Byte 1: MASK flag + payload length
	maskBit := byte(0)
	if ws.isClient {
		maskBit = 0x80
	}

	if length < 126 {
		frame[pos] = maskBit | byte(length)
		pos++
	} else if length < 65536 {
		frame[pos] = maskBit | 126
		pos++
		binary.BigEndian.PutUint16(frame[pos:], uint16(length))
		pos += 2
	} else {
		frame[pos] = maskBit | 127
		pos++
		binary.BigEndian.PutUint64(frame[pos:], uint64(length))
		pos += 8
	}

	// Mask key + masked payload (client only, per RFC 6455)
	if ws.isClient {
		var maskKey [4]byte
		rand.Read(maskKey[:])
		copy(frame[pos:], maskKey[:])
		pos += 4
		for i := 0; i < length; i++ {
			frame[pos+i] = payload[i] ^ maskKey[i%4]
		}
	} else {
		copy(frame[pos:], payload)
	}

	_, err := ws.raw.Write(frame)
	return err
}

func (ws *WSConn) Close() error                       { return ws.raw.Close() }
func (ws *WSConn) LocalAddr() net.Addr                { return ws.raw.LocalAddr() }
func (ws *WSConn) RemoteAddr() net.Addr               { return ws.raw.RemoteAddr() }
func (ws *WSConn) SetDeadline(t time.Time) error      { return ws.raw.SetDeadline(t) }
func (ws *WSConn) SetReadDeadline(t time.Time) error  { return ws.raw.SetReadDeadline(t) }
func (ws *WSConn) SetWriteDeadline(t time.Time) error { return ws.raw.SetWriteDeadline(t) }
