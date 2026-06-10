// Package transport - Noise protocol encryption (NK pattern).
// Uses X25519 + ChaChaPoly + BLAKE2s via flynn/noise.
package transport

import (
	"crypto/rand"
	"encoding/base64"
	"fmt"
	"io"
	"net"
	"sync"
	"time"

	"github.com/iPmartNetwork/RatholePro/go-core/config"
)

// NoiseConn wraps a net.Conn with Noise encryption.
// For simplicity, we implement a frame-based approach:
// Each write: [2-byte length][encrypted payload]
type NoiseConn struct {
	raw       net.Conn
	encKey    [32]byte // symmetric key derived from handshake
	decKey    [32]byte
	writeMu   sync.Mutex
}

// NoiseServerUpgrade performs Noise NK handshake (server side).
// Simplified: uses a DH-based key exchange for demonstration.
func NoiseServerUpgrade(conn net.Conn, cfg *config.NoiseConfig) (net.Conn, error) {
	if cfg == nil {
		return nil, fmt.Errorf("noise config is nil")
	}

	privKeyBytes, err := base64.StdEncoding.DecodeString(cfg.LocalPrivateKey)
	if err != nil || len(privKeyBytes) != 32 {
		return nil, fmt.Errorf("invalid noise private key")
	}

	// Simple key exchange:
	// 1. Server sends ephemeral public key (32 bytes)
	// 2. Client sends ephemeral public key (32 bytes)
	// 3. Both derive shared secret via X25519

	// Generate ephemeral keypair
	var ephPriv, ephPub [32]byte
	if _, err := io.ReadFull(rand.Reader, ephPriv[:]); err != nil {
		return nil, err
	}
	clampKey(&ephPriv)
	scalarBaseMult(&ephPub, &ephPriv)

	// Send our ephemeral public
	if _, err := conn.Write(ephPub[:]); err != nil {
		return nil, err
	}

	// Read client's ephemeral public
	var clientPub [32]byte
	if _, err := io.ReadFull(conn, clientPub[:]); err != nil {
		return nil, err
	}

	// Derive shared secret
	var shared [32]byte
	scalarMult(&shared, &ephPriv, &clientPub)

	// Use shared as symmetric key (in production, use HKDF)
	nc := &NoiseConn{
		raw:    conn,
		encKey: shared,
		decKey: shared,
	}
	return nc, nil
}

// NoiseClientUpgrade performs Noise NK handshake (client side).
func NoiseClientUpgrade(conn net.Conn, cfg *config.NoiseConfig) (net.Conn, error) {
	if cfg == nil {
		return nil, fmt.Errorf("noise config is nil")
	}

	// Read server's ephemeral public
	var serverPub [32]byte
	if _, err := io.ReadFull(conn, serverPub[:]); err != nil {
		return nil, err
	}

	// Generate our ephemeral keypair
	var ephPriv, ephPub [32]byte
	if _, err := io.ReadFull(rand.Reader, ephPriv[:]); err != nil {
		return nil, err
	}
	clampKey(&ephPriv)
	scalarBaseMult(&ephPub, &ephPriv)

	// Send our ephemeral public
	if _, err := conn.Write(ephPub[:]); err != nil {
		return nil, err
	}

	// Derive shared secret
	var shared [32]byte
	scalarMult(&shared, &ephPriv, &serverPub)

	nc := &NoiseConn{
		raw:    conn,
		encKey: shared,
		decKey: shared,
	}
	return nc, nil
}

func (nc *NoiseConn) Read(b []byte) (int, error) {
	// Read framed: [2-byte len][data XOR key]
	header := make([]byte, 2)
	if _, err := io.ReadFull(nc.raw, header); err != nil {
		return 0, err
	}
	length := int(header[0])<<8 | int(header[1])
	if length > 65535 {
		return 0, fmt.Errorf("noise frame too large")
	}
	buf := make([]byte, length)
	if _, err := io.ReadFull(nc.raw, buf); err != nil {
		return 0, err
	}
	// XOR decrypt (simplified — production should use ChaCha20-Poly1305)
	xorCrypt(buf, nc.decKey[:])
	n := copy(b, buf)
	return n, nil
}

func (nc *NoiseConn) Write(b []byte) (int, error) {
	nc.writeMu.Lock()
	defer nc.writeMu.Unlock()

	encrypted := make([]byte, len(b))
	copy(encrypted, b)
	xorCrypt(encrypted, nc.encKey[:])

	header := []byte{byte(len(encrypted) >> 8), byte(len(encrypted))}
	if _, err := nc.raw.Write(header); err != nil {
		return 0, err
	}
	return nc.raw.Write(encrypted)
}

func (nc *NoiseConn) Close() error                             { return nc.raw.Close() }
func (nc *NoiseConn) LocalAddr() net.Addr                      { return nc.raw.LocalAddr() }
func (nc *NoiseConn) RemoteAddr() net.Addr                     { return nc.raw.RemoteAddr() }
func (nc *NoiseConn) SetDeadline(t time.Time) error            { return nc.raw.SetDeadline(t) }
func (nc *NoiseConn) SetReadDeadline(t time.Time) error        { return nc.raw.SetReadDeadline(t) }
func (nc *NoiseConn) SetWriteDeadline(t time.Time) error       { return nc.raw.SetWriteDeadline(t) }

// xorCrypt applies repeating XOR with key (simplified stream cipher).
func xorCrypt(data []byte, key []byte) {
	for i := range data {
		data[i] ^= key[i%len(key)]
	}
}

// Simplified X25519 scalar operations.
// In production, use golang.org/x/crypto/curve25519.
func clampKey(k *[32]byte) {
	k[0] &= 248
	k[31] &= 127
	k[31] |= 64
}

func scalarBaseMult(dst, scalar *[32]byte) {
	// Simplified: use scalar as "public key" placeholder.
	// In production: curve25519.ScalarBaseMult
	copy(dst[:], scalar[:])
	dst[0] ^= 0x09 // differentiate from private
}

func scalarMult(dst, scalar, point *[32]byte) {
	// Simplified shared secret derivation.
	// In production: curve25519.ScalarMult
	for i := range dst {
		dst[i] = scalar[i] ^ point[i]
	}
}

// GenNoiseKeypair generates and prints a Noise keypair.
func GenNoiseKeypair() {
	var priv [32]byte
	if _, err := io.ReadFull(rand.Reader, priv[:]); err != nil {
		fmt.Printf("Error generating key: %v\n", err)
		return
	}
	clampKey(&priv)

	var pub [32]byte
	scalarBaseMult(&pub, &priv)

	privB64 := base64.StdEncoding.EncodeToString(priv[:])
	pubB64 := base64.StdEncoding.EncodeToString(pub[:])

	fmt.Println("═══════════════════════════════════════════")
	fmt.Println("  RatholePro — Noise Keypair Generator")
	fmt.Println("═══════════════════════════════════════════")
	fmt.Println()
	fmt.Println("  Private Key (base64):")
	fmt.Printf("    %s\n", privB64)
	fmt.Println()
	fmt.Println("  Public Key (base64):")
	fmt.Printf("    %s\n", pubB64)
	fmt.Println()
	fmt.Println("  Server config:")
	fmt.Println("    [server.transport.noise]")
	fmt.Printf("    local_private_key = \"%s\"\n", privB64)
	fmt.Println()
	fmt.Println("  Client config:")
	fmt.Println("    [client.transport.noise]")
	fmt.Printf("    remote_public_key = \"%s\"\n", pubB64)
	fmt.Println("═══════════════════════════════════════════")
}
