// Package transport provides pluggable transport layers: TCP, TLS, Noise, WebSocket.
package transport

import (
	"crypto/tls"
	"crypto/x509"
	"fmt"
	"net"
	"os"
	"time"

	"github.com/iPmartNetwork/RatholePro/go-core/config"
)

// Listener wraps net.Listener with transport awareness.
type Listener interface {
	Accept() (net.Conn, error)
	Close() error
	Addr() net.Addr
}

// --- TCP (plain) ---

func ListenTCP(addr string) (net.Listener, error) {
	return net.Listen("tcp", addr)
}

func DialTCP(addr string, timeout time.Duration) (net.Conn, error) {
	return net.DialTimeout("tcp", addr, timeout)
}

// --- TLS ---

func ListenTLS(addr string, cfg *config.TLSConfig) (net.Listener, error) {
	if cfg == nil {
		return nil, fmt.Errorf("TLS config is nil")
	}

	certFile := cfg.TrustedRoot
	keyFile := cfg.PKCS12

	// Auto-generate certificate if enabled
	if cfg.AutoCert {
		autoCfg := &AutoCertConfig{
			CertDir: cfg.CertDir,
			Hosts:   cfg.Hosts,
		}
		result, err := EnsureCert(autoCfg)
		if err != nil {
			return nil, fmt.Errorf("auto-cert: %w", err)
		}
		certFile = result.CertPath
		keyFile = result.KeyPath
	}

	if certFile == "" || keyFile == "" {
		return nil, fmt.Errorf("TLS requires cert and key paths (or set auto_cert = true)")
	}

	cert, err := tls.LoadX509KeyPair(certFile, keyFile)
	if err != nil {
		return nil, fmt.Errorf("load TLS cert/key: %w", err)
	}

	tlsCfg := &tls.Config{
		Certificates: []tls.Certificate{cert},
		MinVersion:   tls.VersionTLS12,
	}

	return tls.Listen("tcp", addr, tlsCfg)
}

func DialTLS(addr string, cfg *config.TLSConfig, timeout time.Duration) (net.Conn, error) {
	if cfg == nil {
		return nil, fmt.Errorf("TLS config is nil")
	}

	tlsCfg := &tls.Config{
		MinVersion: tls.VersionTLS12,
	}

	if cfg.Hostname != "" {
		tlsCfg.ServerName = cfg.Hostname
	}

	// Load custom CA if specified
	if cfg.TrustedRoot != "" {
		caCert, err := os.ReadFile(cfg.TrustedRoot)
		if err != nil {
			return nil, fmt.Errorf("read CA cert: %w", err)
		}
		pool := x509.NewCertPool()
		if !pool.AppendCertsFromPEM(caCert) {
			return nil, fmt.Errorf("failed to parse CA cert")
		}
		tlsCfg.RootCAs = pool
	} else {
		// No trusted_root specified — skip verification (self-signed mode).
		// Encryption is still active, just no CA identity check.
		tlsCfg.InsecureSkipVerify = true
	}

	dialer := &net.Dialer{Timeout: timeout}
	return tls.DialWithDialer(dialer, "tcp", addr, tlsCfg)
}

// --- WebSocket ---
// WebSocket is handled at a higher layer (wrapping the TCP/TLS conn).
// See websocket package.

// --- Factory ---

// ServerListen creates a listener based on transport config.
func ServerListen(addr string, t *config.TransportConfig) (net.Listener, error) {
	typ := config.GetTransportType(t)
	switch typ {
	case "tcp", "ws":
		// For WebSocket, we still listen on TCP; WS upgrade happens per-connection.
		return ListenTCP(addr)
	case "tls":
		if t == nil || t.TLS == nil {
			return nil, fmt.Errorf("TLS transport requires [transport.tls] config")
		}
		return ListenTLS(addr, t.TLS)
	case "noise":
		// Noise uses TCP underneath; encryption is applied per-connection.
		return ListenTCP(addr)
	default:
		return nil, fmt.Errorf("unknown transport type: %s", typ)
	}
}

// ClientDial creates a connection based on transport config.
func ClientDial(addr string, t *config.TransportConfig, timeout time.Duration) (net.Conn, error) {
	typ := config.GetTransportType(t)
	switch typ {
	case "tcp", "ws":
		return DialTCP(addr, timeout)
	case "tls":
		if t == nil || t.TLS == nil {
			return nil, fmt.Errorf("TLS transport requires [transport.tls] config")
		}
		return DialTLS(addr, t.TLS, timeout)
	case "noise":
		// Noise wraps plain TCP; handshake is done after dial.
		return DialTCP(addr, timeout)
	default:
		return nil, fmt.Errorf("unknown transport type: %s", typ)
	}
}
