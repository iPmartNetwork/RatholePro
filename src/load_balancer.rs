//! Load balancing module for distributing connections across multiple backends.
//!
//! Supported strategies:
//! - Round Robin: sequential distribution
//! - Random: random backend selection
//! - Least Connections: route to backend with fewest active connections

use anyhow::Result;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::Mutex;

/// Load balancing strategy
#[derive(Debug, Clone)]
pub enum Strategy {
    RoundRobin,
    Random,
    LeastConnections,
}

impl Strategy {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "random" => Strategy::Random,
            "least_conn" | "least_connections" => Strategy::LeastConnections,
            _ => Strategy::RoundRobin, // default
        }
    }
}

/// A backend server entry
#[derive(Debug, Clone)]
pub struct Backend {
    pub addr: String,
    pub healthy: bool,
    pub active_connections: usize,
}

/// Load balancer managing multiple backends
pub struct LoadBalancer {
    backends: Arc<Mutex<Vec<Backend>>>,
    strategy: Strategy,
    rr_index: AtomicUsize,
}

impl LoadBalancer {
    /// Create a new load balancer with the given backends and strategy
    pub fn new(addrs: Vec<String>, strategy: Strategy) -> Self {
        let backends: Vec<Backend> = addrs
            .into_iter()
            .map(|addr| Backend {
                addr,
                healthy: true,
                active_connections: 0,
            })
            .collect();

        Self {
            backends: Arc::new(Mutex::new(backends)),
            strategy,
            rr_index: AtomicUsize::new(0),
        }
    }

    /// Select the next backend based on strategy
    pub async fn next_backend(&self) -> Result<String> {
        let mut backends = self.backends.lock().await;
        let healthy: Vec<usize> = backends
            .iter()
            .enumerate()
            .filter(|(_, b)| b.healthy)
            .map(|(i, _)| i)
            .collect();

        if healthy.is_empty() {
            return Err(anyhow::anyhow!("No healthy backends available"));
        }

        let idx = match self.strategy {
            Strategy::RoundRobin => {
                let current = self.rr_index.fetch_add(1, Ordering::Relaxed);
                healthy[current % healthy.len()]
            }
            Strategy::Random => {
                use std::time::SystemTime;
                let seed = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .subsec_nanos() as usize;
                healthy[seed % healthy.len()]
            }
            Strategy::LeastConnections => {
                let mut min_conn = usize::MAX;
                let mut min_idx = healthy[0];
                for &i in &healthy {
                    if backends[i].active_connections < min_conn {
                        min_conn = backends[i].active_connections;
                        min_idx = i;
                    }
                }
                min_idx
            }
        };

        backends[idx].active_connections += 1;
        Ok(backends[idx].addr.clone())
    }

    /// Connect to the next available backend
    pub async fn connect(&self) -> Result<(TcpStream, String)> {
        let addr = self.next_backend().await?;
        let stream = TcpStream::connect(&addr).await
            .map_err(|e| {
                tracing::warn!("Backend '{}' connection failed: {}", addr, e);
                anyhow::anyhow!("Failed to connect to backend '{}': {}", addr, e)
            })?;
        stream.set_nodelay(true)?;
        Ok((stream, addr))
    }

    /// Release a connection (decrement active count)
    pub async fn release(&self, addr: &str) {
        let mut backends = self.backends.lock().await;
        if let Some(backend) = backends.iter_mut().find(|b| b.addr == addr) {
            backend.active_connections = backend.active_connections.saturating_sub(1);
        }
    }

    /// Mark a backend as unhealthy
    pub async fn mark_unhealthy(&self, addr: &str) {
        let mut backends = self.backends.lock().await;
        if let Some(backend) = backends.iter_mut().find(|b| b.addr == addr) {
            backend.healthy = false;
            tracing::warn!("Backend '{}' marked unhealthy", addr);
        }
    }

    /// Mark a backend as healthy
    pub async fn mark_healthy(&self, addr: &str) {
        let mut backends = self.backends.lock().await;
        if let Some(backend) = backends.iter_mut().find(|b| b.addr == addr) {
            backend.healthy = true;
            tracing::info!("Backend '{}' marked healthy", addr);
        }
    }

    /// Run health checks periodically
    pub async fn run_health_checks(&self, interval_secs: u64) {
        if interval_secs == 0 {
            return; // Health checks disabled
        }

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;

            let addrs: Vec<String> = {
                let backends = self.backends.lock().await;
                backends.iter().map(|b| b.addr.clone()).collect()
            };

            for addr in addrs {
                let healthy = TcpStream::connect(&addr).await.is_ok();
                if healthy {
                    self.mark_healthy(&addr).await;
                } else {
                    self.mark_unhealthy(&addr).await;
                }
            }
        }
    }

    /// Get current status of all backends
    pub async fn status(&self) -> Vec<Backend> {
        self.backends.lock().await.clone()
    }
}
