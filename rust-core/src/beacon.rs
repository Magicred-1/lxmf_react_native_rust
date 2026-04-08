//! Beacon announce/discovery — matches the anon0mesh CLI protocol
//!
//! Beacon flow:
//!   1. Beacon nodes announce with app_data = b"anonmesh::beacon::v1"
//!   2. Clients register announce handlers that filter by app_data
//!   3. On discovery, clients add the beacon to their pool
//!   4. BeaconPool manages multiple links with race/fallback dispatch
//!
//! The Rust layer tracks beacon state; the actual RNS announce mechanism
//! is handled by the rns-embedded-ffi node internally. This module manages
//! the higher-level beacon pool and reconnection logic.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::node::{DestHash, LxmfEvent};

pub const ANNOUNCE_DATA: &[u8] = b"anonmesh::beacon::v1";

/// Beacon connection state
#[derive(Debug, Clone, PartialEq)]
pub enum BeaconState {
    Discovered,
    Connecting,
    Connected,
    Disconnected,
    Failed,
}

/// A known beacon peer
#[derive(Debug, Clone)]
pub struct Beacon {
    pub dest_hash: DestHash,
    pub state: BeaconState,
    pub last_announce: Instant,
    pub last_connected: Option<Instant>,
    pub reconnect_attempts: u32,
    pub latency_ms: Option<u64>,
}

/// Dispatch strategy for sending requests to beacons
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DispatchStrategy {
    /// Send to all beacons, first valid response wins
    Race,
    /// Try beacons sequentially until one succeeds
    Fallback,
}

/// Manages beacon discovery, connection pool, and announce scheduling
pub struct BeaconManager {
    beacons: HashMap<DestHash, Beacon>,
    strategy: DispatchStrategy,
    announce_active: bool,
    last_announce: Option<Instant>,
    announce_count: u32,
    pending_events: Vec<LxmfEvent>,
}

/// Exponential backoff schedule for reconnection (seconds)
const BACKOFF_SCHEDULE: &[u64] = &[5, 10, 20, 40, 60, 120, 300];

/// Announce burst phase: every 15s for first 2 minutes
const ANNOUNCE_BURST_INTERVAL: Duration = Duration::from_secs(15);
const ANNOUNCE_BURST_DURATION: Duration = Duration::from_secs(120);
/// Steady-state announce interval
const ANNOUNCE_STEADY_INTERVAL: Duration = Duration::from_secs(300);

impl BeaconManager {
    pub fn new() -> Self {
        Self {
            beacons: HashMap::new(),
            strategy: DispatchStrategy::Race,
            announce_active: false,
            last_announce: None,
            announce_count: 0,
            pending_events: Vec::new(),
        }
    }

    /// Start the announce schedule (called when node starts)
    pub fn start_announce_schedule(&mut self) {
        self.announce_active = true;
        self.last_announce = Some(Instant::now());
        self.announce_count = 0;
    }

    /// Stop announcing
    pub fn stop(&mut self) {
        self.announce_active = false;
    }

    /// Check if an announce should be sent now (called from poll loop)
    pub fn should_announce(&self) -> bool {
        if !self.announce_active {
            return false;
        }

        let Some(last) = self.last_announce else {
            return true;
        };

        let elapsed = last.elapsed();
        let start_time = Instant::now() - Duration::from_secs(self.announce_count as u64 * 15);

        if elapsed < ANNOUNCE_BURST_DURATION {
            elapsed >= ANNOUNCE_BURST_INTERVAL
        } else {
            elapsed >= ANNOUNCE_STEADY_INTERVAL
        }
    }

    /// Record that an announce was sent
    pub fn did_announce(&mut self) {
        self.last_announce = Some(Instant::now());
        self.announce_count += 1;
    }

    /// Handle a received announce from another node
    pub fn on_announce_received(&mut self, dest_hash: DestHash, app_data: &[u8]) {
        // Only accept anon0mesh beacon announces
        if app_data != ANNOUNCE_DATA {
            return;
        }

        let now = Instant::now();

        if let Some(beacon) = self.beacons.get_mut(&dest_hash) {
            // Known beacon re-announced — refresh identity, reset backoff
            beacon.last_announce = now;
            beacon.reconnect_attempts = 0;
            if beacon.state == BeaconState::Disconnected || beacon.state == BeaconState::Failed {
                beacon.state = BeaconState::Connecting;
            }
        } else {
            // New beacon discovered
            self.beacons.insert(dest_hash, Beacon {
                dest_hash,
                state: BeaconState::Discovered,
                last_announce: now,
                last_connected: None,
                reconnect_attempts: 0,
                latency_ms: None,
            });

            self.pending_events.push(LxmfEvent::BeaconDiscovered {
                dest_hash,
                app_data: app_data.to_vec(),
            });
        }
    }

    /// Mark a beacon as connected
    pub fn on_beacon_connected(&mut self, dest_hash: &DestHash) {
        if let Some(beacon) = self.beacons.get_mut(dest_hash) {
            beacon.state = BeaconState::Connected;
            beacon.last_connected = Some(Instant::now());
            beacon.reconnect_attempts = 0;
        }
    }

    /// Mark a beacon as disconnected, schedule reconnect
    pub fn on_beacon_disconnected(&mut self, dest_hash: &DestHash) {
        if let Some(beacon) = self.beacons.get_mut(dest_hash) {
            beacon.state = BeaconState::Disconnected;
        }
    }

    /// Get the reconnect delay for a beacon (exponential backoff)
    pub fn reconnect_delay(&self, dest_hash: &DestHash) -> Duration {
        let attempts = self.beacons.get(dest_hash)
            .map(|b| b.reconnect_attempts as usize)
            .unwrap_or(0);

        let idx = attempts.min(BACKOFF_SCHEDULE.len() - 1);
        Duration::from_secs(BACKOFF_SCHEDULE[idx])
    }

    /// Increment reconnect attempt counter
    pub fn on_reconnect_attempt(&mut self, dest_hash: &DestHash) {
        if let Some(beacon) = self.beacons.get_mut(dest_hash) {
            beacon.reconnect_attempts += 1;
            beacon.state = BeaconState::Connecting;
        }
    }

    /// Get all connected beacon destination hashes
    pub fn connected_beacons(&self) -> Vec<DestHash> {
        self.beacons.values()
            .filter(|b| b.state == BeaconState::Connected)
            .map(|b| b.dest_hash)
            .collect()
    }

    /// Get all known beacons with their state
    pub fn all_beacons(&self) -> Vec<&Beacon> {
        self.beacons.values().collect()
    }

    /// Get beacon count
    pub fn beacon_count(&self) -> usize {
        self.beacons.len()
    }

    /// Get connected beacon count
    pub fn connected_count(&self) -> usize {
        self.beacons.values().filter(|b| b.state == BeaconState::Connected).count()
    }

    /// Set dispatch strategy
    pub fn set_strategy(&mut self, strategy: DispatchStrategy) {
        self.strategy = strategy;
    }

    /// Get current dispatch strategy
    pub fn strategy(&self) -> DispatchStrategy {
        self.strategy
    }

    /// Remove a beacon from the pool
    pub fn remove_beacon(&mut self, dest_hash: &DestHash) {
        self.beacons.remove(dest_hash);
    }

    /// Drain pending events for the native layer
    pub fn drain_events(&mut self) -> Vec<LxmfEvent> {
        std::mem::take(&mut self.pending_events)
    }

    /// Get beacons as JSON for TypeScript consumption
    pub fn beacons_json(&self) -> String {
        let beacons: Vec<serde_json::Value> = self.beacons.values().map(|b| {
            serde_json::json!({
                "destHash": hex::encode(b.dest_hash),
                "state": match b.state {
                    BeaconState::Discovered => "discovered",
                    BeaconState::Connecting => "connecting",
                    BeaconState::Connected => "connected",
                    BeaconState::Disconnected => "disconnected",
                    BeaconState::Failed => "failed",
                },
                "reconnectAttempts": b.reconnect_attempts,
                "latencyMs": b.latency_ms,
            })
        }).collect();

        serde_json::to_string(&beacons).unwrap_or_else(|_| "[]".to_string())
    }
}

impl Default for BeaconManager {
    fn default() -> Self {
        Self::new()
    }
}
