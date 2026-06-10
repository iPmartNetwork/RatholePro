package config

import (
	"fmt"
	"os"

	"github.com/BurntSushi/toml"
)

// Config is the top-level configuration, compatible with the existing TOML format.
type Config struct {
	Server *ServerConfig `toml:"server"`
	Client *ClientConfig `toml:"client"`
}

type ServerConfig struct {
	BindAddr          string                    `toml:"bind_addr"`
	DefaultToken      string                    `toml:"default_token"`
	HeartbeatInterval int                       `toml:"heartbeat_interval"`
	Transport         *TransportConfig          `toml:"transport"`
	Services          map[string]*ServiceConfig `toml:"services"`
}

type ClientConfig struct {
	RemoteAddr       string                    `toml:"remote_addr"`
	DefaultToken     string                    `toml:"default_token"`
	HeartbeatTimeout int                       `toml:"heartbeat_timeout"`
	RetryInterval    int                       `toml:"retry_interval"`
	MuxConnections   int                       `toml:"mux_connections"`
	Transport        *TransportConfig          `toml:"transport"`
	Services         map[string]*ServiceConfig `toml:"services"`
}

type TransportConfig struct {
	Type      string           `toml:"type"` // "tcp" (default), "tls", "noise", "ws"
	TLS       *TLSConfig       `toml:"tls"`
	Noise     *NoiseConfig     `toml:"noise"`
	WebSocket *WebSocketConfig `toml:"websocket"`
}

type TLSConfig struct {
	TrustedRoot string   `toml:"trusted_root"`
	PKCS12      string   `toml:"pkcs12"`
	Hostname    string   `toml:"hostname"`
	AutoCert    bool     `toml:"auto_cert"`    // Auto-generate self-signed cert
	CertDir     string   `toml:"cert_dir"`     // Directory for auto-generated certs
	Hosts       []string `toml:"hosts"`        // SANs for auto-generated cert
}

type NoiseConfig struct {
	Pattern         string `toml:"pattern"`
	LocalPrivateKey string `toml:"local_private_key"`
	RemotePublicKey string `toml:"remote_public_key"`
}

type WebSocketConfig struct {
	Path string `toml:"path"`
}

type ServiceConfig struct {
	Type          string `toml:"type"`
	Token         string `toml:"token"`
	BindAddr      string `toml:"bind_addr"`
	LocalAddr     string `toml:"local_addr"`
	Nodelay       bool   `toml:"nodelay"`
	MuxStreams    int    `toml:"mux_streams"`
	MaxMuxStreams int    `toml:"max_mux_streams"`
	RetryInterval int    `toml:"retry_interval"`
}

// GetTransportType returns the effective transport type (defaults to "tcp").
func GetTransportType(t *TransportConfig) string {
	if t == nil || t.Type == "" {
		return "tcp"
	}
	return t.Type
}

type RunMode int

const (
	ModeServer RunMode = iota
	ModeClient
)

func Load(path string) (*Config, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("cannot read '%s': %w", path, err)
	}
	var cfg Config
	if err := toml.Unmarshal(data, &cfg); err != nil {
		return nil, fmt.Errorf("parse error '%s': %w", path, err)
	}
	if cfg.Server == nil && cfg.Client == nil {
		return nil, fmt.Errorf("config must have [server] or [client] section")
	}
	return &cfg, nil
}

func DetermineMode(cfg *Config, forceServer, forceClient bool) RunMode {
	if forceServer {
		return ModeServer
	}
	if forceClient {
		return ModeClient
	}
	if cfg.Server != nil && cfg.Client == nil {
		return ModeServer
	}
	if cfg.Client != nil && cfg.Server == nil {
		return ModeClient
	}
	return ModeServer
}

// GetServiceToken returns the effective token for a service.
func GetServiceToken(svcToken, defaultToken string) string {
	if svcToken != "" {
		return svcToken
	}
	return defaultToken
}
