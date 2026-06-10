package relay

import (
	"io"
	"net"
	"sync"
)

const bufSize = 64 * 1024 // 64KB buffer for high throughput

// Relay performs transparent bidirectional TCP copy between two connections.
func Relay(a, b net.Conn) {
	var wg sync.WaitGroup
	wg.Add(2)

	go func() {
		defer wg.Done()
		buf := make([]byte, bufSize)
		_, _ = io.CopyBuffer(a, b, buf)
		if tc, ok := a.(*net.TCPConn); ok {
			_ = tc.CloseWrite()
		}
	}()

	go func() {
		defer wg.Done()
		buf := make([]byte, bufSize)
		_, _ = io.CopyBuffer(b, a, buf)
		if tc, ok := b.(*net.TCPConn); ok {
			_ = tc.CloseWrite()
		}
	}()

	wg.Wait()
	_ = a.Close()
	_ = b.Close()
}
