//! Load balancing module.
//! Distributes connections across multiple backends.
//! Strategies: round_robin, random, least_conn.

use anyhow::Result;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::net::TcpStream;

#[derive(Debug, Clone)]
pub enum Strategy { RoundRobin, Random, LeastConn }

impl Strategy {
    pub fn from_str(s: &str) -> Self {
        match s {
            "random" => Self::Random,
            "least_conn" => Self::LeastConn,
            _ => Self::RoundRobin,
        }
    }
}

pub struct LoadBalancer {
    backends: Vec<String>,
    strategy: Strategy,
    index: AtomicUsize,
    connections: Vec<AtomicUsize>,
}

impl LoadBalancer {
    pub fn new(backends: Vec<String>, strategy: Strategy) -> Self {
        let conns = backends.iter().map(|_| AtomicUsize::new(0)).collect();
        Self { backends, strategy, index: AtomicUsize::new(0), connections: conns }
    }

    /// Pick next backend address
    pub fn next(&self) -> Option<&str> {
        if self.backends.is_empty() { return None; }
        let idx = match self.strategy {
            Strategy::RoundRobin => {
                self.index.fetch_add(1, Ordering::Relaxed) % self.backends.len()
            }
            Strategy::Random => {
                let t = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                (t.subsec_nanos() as usize) % self.backends.len()
            }
            Strategy::LeastConn => {
                let mut min = usize::MAX;
                let mut mi = 0;
                for (i, c) in self.connections.iter().enumerate() {
                    let v = c.load(Ordering::Relaxed);
                    if v < min { min = v; mi = i; }
                }
                mi
            }
        };
        Some(&self.backends[idx])
    }

    /// Connect to next backend
    pub async fn connect(&self) -> Result<(TcpStream, usize)> {
        let idx = match self.strategy {
            Strategy::RoundRobin => self.index.fetch_add(1, Ordering::Relaxed) % self.backends.len(),
            Strategy::Random => {
                let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                (t.subsec_nanos() as usize) % self.backends.len()
            }
            Strategy::LeastConn => {
                let mut min = usize::MAX; let mut mi = 0;
                for (i, c) in self.connections.iter().enumerate() { let v = c.load(Ordering::Relaxed); if v < min { min = v; mi = i; } }
                mi
            }
        };
        let addr = &self.backends[idx];
        self.connections[idx].fetch_add(1, Ordering::Relaxed);
        let stream = TcpStream::connect(addr).await
            .map_err(|e| anyhow::anyhow!("Backend '{}' failed: {}", addr, e))?;
        let _ = stream.set_nodelay(true);
        Ok((stream, idx))
    }

    /// Release connection
    pub fn release(&self, idx: usize) {
        if idx < self.connections.len() {
            self.connections[idx].fetch_sub(1, Ordering::Relaxed);
        }
    }
}
