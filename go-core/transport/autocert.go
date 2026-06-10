// Package transport - Auto-generate self-signed TLS certificates.
// Useful for internal tunnels that need encryption without buying a CA cert.
package transport

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/pem"
	"fmt"
	"log"
	"math/big"
	"net"
	"os"
	"path/filepath"
	"time"
)

const (
	defaultCertValidDays = 365 * 3 // 3 years
	defaultCertDir       = "certs"
	certFileName         = "rathole-pro.crt"
	keyFileName          = "rathole-pro.key"
)

// AutoCertConfig holds options for automatic certificate generation.
type AutoCertConfig struct {
	CertDir    string   // Directory to store cert/key (default: ./certs)
	Hosts      []string // Hostnames and IPs for the certificate
	ValidDays  int      // Validity period in days (default: 3 years)
	ForceRegen bool     // Force regeneration even if files exist
}

// AutoCertResult contains paths to the generated cert and key.
type AutoCertResult struct {
	CertPath string
	KeyPath  string
	IsNew    bool // true if newly generated, false if already existed
}

// EnsureCert checks if cert/key exist; if not, generates them automatically.
func EnsureCert(cfg *AutoCertConfig) (*AutoCertResult, error) {
	if cfg == nil {
		cfg = &AutoCertConfig{}
	}

	// Defaults
	if cfg.CertDir == "" {
		cfg.CertDir = defaultCertDir
	}
	if cfg.ValidDays <= 0 {
		cfg.ValidDays = defaultCertValidDays
	}
	if len(cfg.Hosts) == 0 {
		// Auto-detect: localhost + all local IPs
		cfg.Hosts = detectLocalHosts()
	}

	certPath := filepath.Join(cfg.CertDir, certFileName)
	keyPath := filepath.Join(cfg.CertDir, keyFileName)

	// Check if already exists
	if !cfg.ForceRegen {
		if fileExists(certPath) && fileExists(keyPath) {
			// Verify cert is still valid
			if isCertValid(certPath) {
				log.Printf("[autocert] using existing cert: %s", certPath)
				return &AutoCertResult{
					CertPath: certPath,
					KeyPath:  keyPath,
					IsNew:    false,
				}, nil
			}
			log.Printf("[autocert] existing cert expired, regenerating")
		}
	}

	// Generate new cert
	log.Printf("[autocert] generating self-signed certificate for %v", cfg.Hosts)

	if err := os.MkdirAll(cfg.CertDir, 0700); err != nil {
		return nil, fmt.Errorf("create cert dir: %w", err)
	}

	if err := generateSelfSigned(certPath, keyPath, cfg.Hosts, cfg.ValidDays); err != nil {
		return nil, err
	}

	log.Printf("[autocert] certificate generated: %s", certPath)
	log.Printf("[autocert] private key generated: %s", keyPath)

	return &AutoCertResult{
		CertPath: certPath,
		KeyPath:  keyPath,
		IsNew:    true,
	}, nil
}

// generateSelfSigned creates a self-signed ECDSA certificate.
func generateSelfSigned(certPath, keyPath string, hosts []string, validDays int) error {
	// Generate ECDSA P-256 private key
	privateKey, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return fmt.Errorf("generate key: %w", err)
	}

	// Serial number
	serialNumber, err := rand.Int(rand.Reader, new(big.Int).Lsh(big.NewInt(1), 128))
	if err != nil {
		return fmt.Errorf("generate serial: %w", err)
	}

	now := time.Now()
	template := &x509.Certificate{
		SerialNumber: serialNumber,
		Subject: pkix.Name{
			Organization: []string{"RatholePro"},
			CommonName:   "RatholePro Auto-Generated",
		},
		NotBefore:             now,
		NotAfter:              now.Add(time.Duration(validDays) * 24 * time.Hour),
		KeyUsage:              x509.KeyUsageKeyEncipherment | x509.KeyUsageDigitalSignature,
		ExtKeyUsage:           []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth, x509.ExtKeyUsageClientAuth},
		BasicConstraintsValid: true,
	}

	// Add SANs (Subject Alternative Names)
	for _, h := range hosts {
		if ip := net.ParseIP(h); ip != nil {
			template.IPAddresses = append(template.IPAddresses, ip)
		} else {
			template.DNSNames = append(template.DNSNames, h)
		}
	}

	// Self-sign
	certDER, err := x509.CreateCertificate(rand.Reader, template, template, &privateKey.PublicKey, privateKey)
	if err != nil {
		return fmt.Errorf("create certificate: %w", err)
	}

	// Write cert PEM
	certFile, err := os.OpenFile(certPath, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0644)
	if err != nil {
		return fmt.Errorf("write cert: %w", err)
	}
	defer certFile.Close()
	if err := pem.Encode(certFile, &pem.Block{Type: "CERTIFICATE", Bytes: certDER}); err != nil {
		return fmt.Errorf("encode cert PEM: %w", err)
	}

	// Write key PEM
	keyBytes, err := x509.MarshalECPrivateKey(privateKey)
	if err != nil {
		return fmt.Errorf("marshal key: %w", err)
	}
	keyFile, err := os.OpenFile(keyPath, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0600)
	if err != nil {
		return fmt.Errorf("write key: %w", err)
	}
	defer keyFile.Close()
	if err := pem.Encode(keyFile, &pem.Block{Type: "EC PRIVATE KEY", Bytes: keyBytes}); err != nil {
		return fmt.Errorf("encode key PEM: %w", err)
	}

	return nil
}

// isCertValid checks if an existing certificate file is still within its validity period.
func isCertValid(certPath string) bool {
	data, err := os.ReadFile(certPath)
	if err != nil {
		return false
	}
	block, _ := pem.Decode(data)
	if block == nil {
		return false
	}
	cert, err := x509.ParseCertificate(block.Bytes)
	if err != nil {
		return false
	}
	// Consider invalid if less than 30 days remaining
	return time.Now().Add(30 * 24 * time.Hour).Before(cert.NotAfter)
}

func fileExists(path string) bool {
	_, err := os.Stat(path)
	return err == nil
}

// detectLocalHosts finds all local IPs + localhost for auto-cert SANs.
func detectLocalHosts() []string {
	hosts := []string{"localhost", "127.0.0.1", "::1"}

	ifaces, err := net.Interfaces()
	if err != nil {
		return hosts
	}
	for _, iface := range ifaces {
		if iface.Flags&net.FlagUp == 0 || iface.Flags&net.FlagLoopback != 0 {
			continue
		}
		addrs, err := iface.Addrs()
		if err != nil {
			continue
		}
		for _, addr := range addrs {
			var ip net.IP
			switch v := addr.(type) {
			case *net.IPNet:
				ip = v.IP
			case *net.IPAddr:
				ip = v.IP
			}
			if ip != nil && !ip.IsLoopback() {
				hosts = append(hosts, ip.String())
			}
		}
	}
	return hosts
}
