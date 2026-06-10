package relay

import (
	"io"
	"net"
	"sync"
)

// Relay performs transparent bidirectional TCP copy between two connections.
// It closes both connections when either direction finishes or errors.
func Relay(a, b net.Conn) {
	var wg sync.WaitGroup
	wg.Add(2)

	copyAndClose := func(dst, src net.Conn) {
		defer wg.Done()
		_, _ = io.Copy(dst, src)
		// Signal the other side we're done reading
		if tc, ok := dst.(*net.TCPConn); ok {
			_ = tc.CloseWrite()
		}
	}

	go copyAndClose(a, b)
	go copyAndClose(b, a)

	wg.Wait()
	_ = a.Close()
	_ = b.Close()
}
