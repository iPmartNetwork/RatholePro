package relay

import (
	"io"
	"net"
	"sync"
)

// Relay performs transparent bidirectional TCP copy between two connections.
func Relay(a, b net.Conn) {
	var wg sync.WaitGroup
	wg.Add(2)

	go func() {
		defer wg.Done()
		_, _ = io.Copy(a, b)
		if tc, ok := a.(*net.TCPConn); ok {
			_ = tc.CloseWrite()
		}
	}()

	go func() {
		defer wg.Done()
		_, _ = io.Copy(b, a)
		if tc, ok := b.(*net.TCPConn); ok {
			_ = tc.CloseWrite()
		}
	}()

	wg.Wait()
	_ = a.Close()
	_ = b.Close()
}
