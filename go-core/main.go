package main

import (
	"flag"
	"fmt"
	"log"
	"os"

	"github.com/iPmartNetwork/RatholePro/go-core/client"
	"github.com/iPmartNetwork/RatholePro/go-core/config"
	"github.com/iPmartNetwork/RatholePro/go-core/server"
	"github.com/iPmartNetwork/RatholePro/go-core/transport"
)

const version = "0.4.2"

func main() {
	log.SetFlags(log.LstdFlags | log.Lshortfile)

	forceServer := flag.Bool("server", false, "Force server mode")
	forceClient := flag.Bool("client", false, "Force client mode")
	shortS := flag.Bool("s", false, "Force server mode (short)")
	shortC := flag.Bool("c", false, "Force client mode (short)")
	validate := flag.Bool("validate", false, "Validate config and exit")
	showVersion := flag.Bool("version", false, "Show version")
	genKey := flag.Bool("gen-key", false, "Generate Noise keypair")

	flag.Usage = func() {
		fmt.Fprintf(os.Stderr, "RatholePro v%s — Transparent TCP/UDP tunnel with yamux multiplexing\n", version)
		fmt.Fprintf(os.Stderr, "Developer: iPmart Network (Ali Hassanzadeh)\n\n")
		fmt.Fprintf(os.Stderr, "Usage: rathole-pro [options] <CONFIG>\n")
		fmt.Fprintf(os.Stderr, "       rathole-pro --gen-key\n\n")
		fmt.Fprintf(os.Stderr, "Options:\n")
		flag.PrintDefaults()
	}

	flag.Parse()

	if *showVersion {
		fmt.Printf("RatholePro v%s (Go core with yamux)\n", version)
		os.Exit(0)
	}

	if *genKey {
		transport.GenNoiseKeypair()
		os.Exit(0)
	}

	args := flag.Args()
	if len(args) < 1 {
		flag.Usage()
		os.Exit(1)
	}

	configPath := args[0]
	log.Printf("RatholePro v%s (Go core)", version)

	cfg, err := config.Load(configPath)
	if err != nil {
		log.Fatalf("Config error: %v", err)
	}

	if *validate {
		fmt.Println("✓ Config OK")
		os.Exit(0)
	}

	isServer := *forceServer || *shortS
	isClient := *forceClient || *shortC
	mode := config.DetermineMode(cfg, isServer, isClient)

	switch mode {
	case config.ModeServer:
		if err := server.Run(cfg); err != nil {
			log.Fatalf("[server] fatal: %v", err)
		}
	case config.ModeClient:
		if err := client.Run(cfg); err != nil {
			log.Fatalf("[client] fatal: %v", err)
		}
	}
}
