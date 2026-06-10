// Package httpproxy implements HTTP CONNECT tunneling and HTTP forwarding.
package httpproxy

import (
	"bufio"
	"fmt"
	"io"
	"log"
	"net"
	"strings"
	"sync"
	"time"
)

// ParseRequestLine parses "METHOD PATH VERSION" from HTTP request.
func ParseRequestLine(data []byte) (method, path, version string, err error) {
	s := string(data)
	line := strings.SplitN(s, "\r\n", 2)[0]
	parts := strings.SplitN(line, " ", 3)
	if len(parts) < 3 {
		return "", "", "", fmt.Errorf("invalid request line")
	}
	return parts[0], parts[1], parts[2], nil
}

// ExtractHost extracts the Host header from raw HTTP request bytes.
func ExtractHost(data []byte) string {
	s := string(data)
	for _, line := range strings.Split(s, "\r\n") {
		lower := strings.ToLower(line)
		if strings.HasPrefix(lower, "host:") {
			return strings.TrimSpace(line[5:])
		}
	}
	return ""
}

// HandleCONNECT handles HTTP CONNECT (HTTPS tunneling).
func HandleCONNECT(client net.Conn, target string) error {
	targetConn, err := net.DialTimeout("tcp", target, 10*time.Second)
	if err != nil {
		resp := "HTTP/1.1 502 Bad Gateway\r\n\r\n"
		_, _ = client.Write([]byte(resp))
		return fmt.Errorf("connect to %s: %w", target, err)
	}

	// Send 200 to client
	_, err = client.Write([]byte("HTTP/1.1 200 Connection Established\r\n\r\n"))
	if err != nil {
		targetConn.Close()
		return err
	}

	// Transparent relay
	var wg sync.WaitGroup
	wg.Add(2)
	go func() {
		defer wg.Done()
		_, _ = io.Copy(targetConn, client)
		if tc, ok := targetConn.(*net.TCPConn); ok {
			_ = tc.CloseWrite()
		}
	}()
	go func() {
		defer wg.Done()
		_, _ = io.Copy(client, targetConn)
		if tc, ok := client.(*net.TCPConn); ok {
			_ = tc.CloseWrite()
		}
	}()
	wg.Wait()
	_ = client.Close()
	_ = targetConn.Close()
	return nil
}

// ForwardRequest forwards an HTTP request to the backend and returns the response.
func ForwardRequest(requestBytes []byte, backend string) ([]byte, error) {
	conn, err := net.DialTimeout("tcp", backend, 10*time.Second)
	if err != nil {
		return nil, fmt.Errorf("connect backend %s: %w", backend, err)
	}
	defer conn.Close()

	if tc, ok := conn.(*net.TCPConn); ok {
		_ = tc.SetNoDelay(true)
	}

	if _, err := conn.Write(requestBytes); err != nil {
		return nil, fmt.Errorf("write to backend: %w", err)
	}

	// Read response
	var response []byte
	buf := make([]byte, 8192)
	for {
		_ = conn.SetReadDeadline(time.Now().Add(5 * time.Second))
		n, err := conn.Read(buf)
		if n > 0 {
			response = append(response, buf[:n]...)
		}
		if err != nil {
			break
		}
	}
	return response, nil
}

// ServeHTTPProxy starts a simple HTTP proxy on the given listener.
func ServeHTTPProxy(ln net.Listener) error {
	log.Printf("[http-proxy] listening on %s", ln.Addr())
	for {
		conn, err := ln.Accept()
		if err != nil {
			return err
		}
		go handleHTTPProxy(conn)
	}
}

func handleHTTPProxy(conn net.Conn) {
	defer conn.Close()

	reader := bufio.NewReader(conn)
	// Read the request
	var requestBuf []byte
	for {
		line, err := reader.ReadBytes('\n')
		if err != nil {
			return
		}
		requestBuf = append(requestBuf, line...)
		if strings.TrimSpace(string(line)) == "" {
			break
		}
	}

	method, target, _, err := ParseRequestLine(requestBuf)
	if err != nil {
		return
	}

	if strings.ToUpper(method) == "CONNECT" {
		// HTTPS tunneling
		if err := HandleCONNECT(conn, target); err != nil {
			log.Printf("[http-proxy] CONNECT %s: %v", target, err)
		}
	} else {
		// HTTP forwarding
		host := ExtractHost(requestBuf)
		if host == "" {
			_, _ = conn.Write([]byte("HTTP/1.1 400 Bad Request\r\n\r\n"))
			return
		}
		if !strings.Contains(host, ":") {
			host += ":80"
		}
		resp, err := ForwardRequest(requestBuf, host)
		if err != nil {
			_, _ = conn.Write([]byte("HTTP/1.1 502 Bad Gateway\r\n\r\n"))
			return
		}
		_, _ = conn.Write(resp)
	}
}
