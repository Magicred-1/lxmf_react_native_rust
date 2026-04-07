//! LxmfNode — high-level wrapper around rns-embedded-ffi V1 API
//!
//! Manages the Reticulum/LXMF node lifecycle, event subscription,
//! and provides a clean Rust API that the FFI and JNI layers export.

use std::sync::{Arc, Mutex, OnceLock};
use std::collections::VecDeque;

use rns_embedded_ffi::{
    RnsEmbeddedV1Node, RnsEmbeddedV1NodeConfig, RnsEmbeddedV1NodeError,
    RnsEmbeddedV1NodeStatus, RnsEmbeddedV1SendReceipt, RnsEmbeddedV1Capabilities,
    RnsEmbeddedV1NodeEvent, RnsEmbeddedV1PollResult, RnsEmbeddedV1PollResultKind,
    RnsEmbeddedV1EventKind, RnsEmbeddedV1LogLevel, RnsEmbeddedEventSubscription,
    RnsEmbeddedStatus, RnsEmbeddedNodeMode, RnsEmbeddedLifecycleState,
    RnsEmbeddedV1NodeErrorCode,
    rns_embedded_v1_node_new, rns_embedded_v1_node_free,
    rns_embedded_v1_node_start, rns_embedded_v1_node_stop, rns_embedded_v1_node_restart,
    rns_embedded_v1_node_get_status, rns_embedded_v1_node_send, rns_embedded_v1_node_broadcast,
    rns_embedded_v1_node_set_log_level, rns_embedded_v1_node_subscribe_events,
    rns_embedded_v1_subscription_next, rns_embedded_v1_subscription_close,
    rns_embedded_v1_node_config_default, rns_embedded_v1_get_capabilities,
    rns_embedded_v1_abi_version,
};

use crate::beacon::BeaconManager;
use crate::store::MessageStore;

/// Destination hash: 16 bytes identifying a Reticulum destination
pub type DestHash = [u8; 16];

/// Identity key: 32 bytes for the node's cryptographic identity
pub type IdentityKey = [u8; 32];

/// LXMF address: 16 bytes
pub type LxmfAddress = [u8; 16];

/// Application constants matching the anon0mesh CLI protocol
pub const APP_NAME: &str = "anon0mesh";
pub const APP_ASPECT_BEACON: &str = "rpc_beacon";
pub const APP_ASPECT_NODE: &str = "node";
pub const ANNOUNCE_DATA: &[u8] = b"anonmesh::beacon::v1";

/// Events emitted to the native layer (Swift/Kotlin) for forwarding to JS
#[derive(Debug, Clone)]
pub enum LxmfEvent {
    /// Node status changed (running, lifecycle state)
    StatusChanged {
        running: bool,
        lifecycle: u32,
    },
    /// Packet received on our SINGLE destination
    PacketReceived {
        source: DestHash,
        data: Vec<u8>,
    },
    /// Solana transaction received on GROUP relay destination
    TxReceived {
        data: Vec<u8>,
    },
    /// Beacon discovered via announce
    BeaconDiscovered {
        dest_hash: DestHash,
        app_data: Vec<u8>,
    },
    /// LXMF message received
    MessageReceived {
        source: LxmfAddress,
        content: Vec<u8>,
        timestamp: u64,
    },
    /// Log message from the node
    Log {
        level: u32,
        message: String,
    },
    /// Error from the node
    Error {
        code: u32,
        message: String,
    },
}

/// Thread-safe event queue for passing events from Rust to native polling
pub type EventQueue = Arc<Mutex<VecDeque<LxmfEvent>>>;

/// Global singleton node instance
static NODE: OnceLock<Mutex<Option<LxmfNode>>> = OnceLock::new();

/// Global bridge instance (for mode 3 = standard Reticulum TCP)
static BRIDGE: OnceLock<Mutex<Option<Arc<crate::reticulum_bridge::ReticulumTcpBridge>>>> = OnceLock::new();

fn bridge_global() -> &'static Mutex<Option<Arc<crate::reticulum_bridge::ReticulumTcpBridge>>> {
    BRIDGE.get_or_init(|| Mutex::new(None))
}

/// The main LXMF mesh node
pub struct LxmfNode {
    /// Opaque pointer to the rns-embedded-ffi V1 node
    raw_node: *mut RnsEmbeddedV1Node,
    /// Event subscription handle
    subscription: Option<*mut RnsEmbeddedEventSubscription>,
    /// Outbound event queue (polled by native layer)
    pub events: EventQueue,
    /// Beacon manager for announce/discovery
    pub beacon_mgr: BeaconManager,
    /// Message persistence
    pub store: Option<MessageStore>,
    /// Whether the node is currently running
    running: bool,
    /// The node's LXMF address (set after start)
    pub lxmf_address: LxmfAddress,
    /// The node's identity key (set after start)
    pub identity: IdentityKey,
}

// Raw pointers are Send-safe here because we control all access through the Mutex
unsafe impl Send for LxmfNode {}

impl LxmfNode {
    /// Get or initialize the global node singleton
    pub fn global() -> &'static Mutex<Option<LxmfNode>> {
        NODE.get_or_init(|| Mutex::new(None))
    }

    /// Initialize a new LXMF node. Does not start it yet.
    pub fn init(db_path: Option<&str>) -> Result<(), String> {
        let node_ptr = unsafe { rns_embedded_v1_node_new() };
        if node_ptr.is_null() {
            return Err("Failed to allocate V1 node".into());
        }

        let store = db_path.map(|p| {
            MessageStore::open(p).map_err(|e| format!("SQLite open failed: {e}"))
        }).transpose()?;

        let node = LxmfNode {
            raw_node: node_ptr,
            subscription: None,
            events: Arc::new(Mutex::new(VecDeque::with_capacity(256))),
            beacon_mgr: BeaconManager::new(),
            store,
            running: false,
            lxmf_address: [0u8; 16],
            identity: [0u8; 32],
        };

        let mut guard = Self::global().lock().map_err(|e| e.to_string())?;
        *guard = Some(node);
        Ok(())
    }

    /// Start the node with the given configuration.
    /// mode: 0=BLE, 1=FFI TcpClient, 2=FFI TcpServer, 3=Standard Reticulum TCP (HDLC bridge)
    pub fn start(
        identity: &IdentityKey,
        lxmf_address: &LxmfAddress,
        mode: u32,
        announce_interval_ms: u64,
        ble_mtu_hint: u16,
        tcp_host: Option<&str>,
        tcp_port: u16,
    ) -> Result<(), String> {
        // Mode 3: use the Reticulum HDLC bridge instead of the V1 managed API
        if mode == 3 {
            let host = tcp_host.ok_or("Reticulum TCP mode requires a tcpHost")?;
            let bridge = crate::reticulum_bridge::ReticulumTcpBridge::start(
                crate::reticulum_bridge::BridgeConfig {
                    identity: *identity,
                    lxmf_address: *lxmf_address,
                    tcp_host: host.to_string(),
                    tcp_port,
                    announce_interval_ms,
                    ble_mtu_hint,
                },
            )?;

            let mut bridge_guard = bridge_global().lock().map_err(|e| e.to_string())?;
            *bridge_guard = Some(bridge);

            // Also mark the V1 node as running for status queries
            let mut guard = Self::global().lock().map_err(|e| e.to_string())?;
            if let Some(node) = guard.as_mut() {
                node.identity = *identity;
                node.lxmf_address = *lxmf_address;
                node.running = true;
            }
            return Ok(());
        }

        let mut guard = Self::global().lock().map_err(|e| e.to_string())?;
        let node = guard.as_mut().ok_or("Node not initialized")?;

        if node.running {
            return Err("Node already running".into());
        }

        let mut config = unsafe { rns_embedded_v1_node_config_default() };
        config.store_identity = *identity;
        config.lxmf_address = *lxmf_address;
        config.node_mode = match mode {
            1 => RnsEmbeddedNodeMode::TcpClient,
            2 => RnsEmbeddedNodeMode::TcpServer,
            _ => RnsEmbeddedNodeMode::BleOnly,
        };
        config.announce_interval_ms = announce_interval_ms;
        config.ble_mtu_hint = ble_mtu_hint;

        if let Some(host) = tcp_host {
            let host_bytes = host.as_bytes();
            let len = host_bytes.len().min(255);
            config.tcp_host[..len].copy_from_slice(&host_bytes[..len]);
            config.tcp_port = tcp_port;
        }

        let mut node_error = default_node_error();

        let status = unsafe {
            rns_embedded_v1_node_start(node.raw_node, &config, &mut node_error)
        };

        if status != RnsEmbeddedStatus::Ok {
            return Err(format!("Start failed: status={status:?}, error_code={:?}", node_error.code));
        }

        // Subscribe to events
        let mut sub_ptr: *mut RnsEmbeddedEventSubscription = std::ptr::null_mut();
        let sub_status = unsafe {
            rns_embedded_v1_node_subscribe_events(node.raw_node, &mut sub_ptr, &mut node_error)
        };

        if sub_status == RnsEmbeddedStatus::Ok && !sub_ptr.is_null() {
            node.subscription = Some(sub_ptr);
        }

        node.identity = *identity;
        node.lxmf_address = *lxmf_address;
        node.running = true;

        // Start beacon announce schedule
        node.beacon_mgr.start_announce_schedule();

        Ok(())
    }

    /// Stop the node
    pub fn stop() -> Result<(), String> {
        let mut guard = Self::global().lock().map_err(|e| e.to_string())?;
        let node = guard.as_mut().ok_or("Node not initialized")?;

        if !node.running {
            return Ok(());
        }

        // Close event subscription
        if let Some(sub) = node.subscription.take() {
            let mut node_error = default_node_error();
            unsafe { rns_embedded_v1_subscription_close(sub, &mut node_error) };
        }

        let mut node_error = default_node_error();
        let status = unsafe { rns_embedded_v1_node_stop(node.raw_node, &mut node_error) };

        node.running = false;
        node.beacon_mgr.stop();

        if status != RnsEmbeddedStatus::Ok {
            return Err(format!("Stop failed: status={status:?}"));
        }

        Ok(())
    }

    /// Send a message to a specific destination (16-byte hash)
    pub fn send(destination: &DestHash, body: &[u8]) -> Result<SendReceipt, String> {
        let mut guard = Self::global().lock().map_err(|e| e.to_string())?;
        let node = guard.as_mut().ok_or("Node not initialized")?;

        if !node.running {
            return Err("Node not running".into());
        }

        let mut receipt = default_send_receipt();
        let mut node_error = default_node_error();

        let status = unsafe {
            rns_embedded_v1_node_send(
                node.raw_node,
                destination.as_ptr(),
                body.as_ptr(),
                body.len(),
                &mut receipt,
                &mut node_error,
            )
        };

        if status != RnsEmbeddedStatus::Ok {
            return Err(format!("Send failed: status={status:?}, error={:?}", node_error.code));
        }

        // Persist outbound message
        if let Some(ref store) = node.store {
            let _ = store.insert_message(destination, body, true);
        }

        Ok(SendReceipt {
            operation_id: receipt.operation_id,
            accepted_bytes: receipt.accepted_bytes,
            queued: receipt.queued,
        })
    }

    /// Broadcast to multiple destinations (e.g., Solana tx relay to all beacons)
    pub fn broadcast(destinations: &[DestHash], body: &[u8]) -> Result<SendReceipt, String> {
        let mut guard = Self::global().lock().map_err(|e| e.to_string())?;
        let node = guard.as_mut().ok_or("Node not initialized")?;

        if !node.running {
            return Err("Node not running".into());
        }

        let flat_dests: Vec<u8> = destinations.iter().flat_map(|d| d.iter().copied()).collect();
        let mut receipt = default_send_receipt();
        let mut node_error = default_node_error();

        let status = unsafe {
            rns_embedded_v1_node_broadcast(
                node.raw_node,
                flat_dests.as_ptr(),
                destinations.len(),
                body.as_ptr(),
                body.len(),
                &mut receipt,
                &mut node_error,
            )
        };

        if status != RnsEmbeddedStatus::Ok {
            return Err(format!("Broadcast failed: status={status:?}"));
        }

        Ok(SendReceipt {
            operation_id: receipt.operation_id,
            accepted_bytes: receipt.accepted_bytes,
            queued: receipt.queued,
        })
    }

    /// Poll for pending events. Called by the native layer's timer loop.
    /// Returns events that should be forwarded to JavaScript.
    pub fn poll_events(timeout_ms: u64) -> Vec<LxmfEvent> {
        let mut guard = match Self::global().lock() {
            Ok(g) => g,
            Err(_) => return vec![],
        };
        let node = match guard.as_mut() {
            Some(n) => n,
            None => return vec![],
        };

        let sub = match node.subscription {
            Some(s) => s,
            None => return vec![],
        };

        let mut events = Vec::new();
        let mut poll_result = default_poll_result();
        let mut event = default_node_event();
        let mut node_error = default_node_error();

        // Drain available events (non-blocking after first)
        loop {
            let timeout = if events.is_empty() { timeout_ms } else { 0 };

            let status = unsafe {
                rns_embedded_v1_subscription_next(
                    sub, timeout, &mut poll_result, &mut event, &mut node_error,
                )
            };

            if status != RnsEmbeddedStatus::Ok {
                break;
            }

            match poll_result.kind {
                RnsEmbeddedV1PollResultKind::Event => {
                    if let Some(lxmf_event) = map_event(&event, node) {
                        events.push(lxmf_event);
                    }
                }
                RnsEmbeddedV1PollResultKind::Timeout => break,
                RnsEmbeddedV1PollResultKind::Closed
                | RnsEmbeddedV1PollResultKind::NodeStopped => {
                    events.push(LxmfEvent::StatusChanged {
                        running: false,
                        lifecycle: 0,
                    });
                    break;
                }
                RnsEmbeddedV1PollResultKind::NodeRestarted => {
                    events.push(LxmfEvent::StatusChanged {
                        running: true,
                        lifecycle: event.lifecycle_state as u32,
                    });
                }
                RnsEmbeddedV1PollResultKind::Gap => {
                    // Sequence gap — log but continue
                    events.push(LxmfEvent::Log {
                        level: 1, // warn
                        message: format!("Event sequence gap at id={}", poll_result.next_event_id),
                    });
                }
            }

            // Don't block forever draining
            if events.len() >= 64 {
                break;
            }
        }

        // Also drain beacon events
        events.extend(node.beacon_mgr.drain_events());

        events
    }

    /// Get current node status
    pub fn get_status() -> Result<NodeStatus, String> {
        let mut guard = Self::global().lock().map_err(|e| e.to_string())?;
        let node = guard.as_mut().ok_or("Node not initialized")?;

        let mut status = default_node_status();
        let status_code = unsafe {
            rns_embedded_v1_node_get_status(node.raw_node, &mut status)
        };

        if status_code != RnsEmbeddedStatus::Ok {
            return Err(format!("Get status failed: {status_code:?}"));
        }

        Ok(NodeStatus {
            running: node.running,
            lifecycle: status.lifecycle_state as u32,
            epoch: status.epoch,
            pending_outbound: status.pending_outbound,
            outbound_sent: status.outbound_sent,
            inbound_accepted: status.inbound_accepted,
            announces_received: status.announces_received,
            lxmf_messages_received: status.lxmf_messages_received,
        })
    }

    /// Set log level
    pub fn set_log_level(level: u32) -> Result<(), String> {
        let mut guard = Self::global().lock().map_err(|e| e.to_string())?;
        let node = guard.as_mut().ok_or("Node not initialized")?;

        let log_level = match level {
            0 => RnsEmbeddedV1LogLevel::Error,
            1 => RnsEmbeddedV1LogLevel::Warn,
            2 => RnsEmbeddedV1LogLevel::Info,
            3 => RnsEmbeddedV1LogLevel::Debug,
            _ => RnsEmbeddedV1LogLevel::Trace,
        };

        let mut node_error = default_node_error();
        let status = unsafe {
            rns_embedded_v1_node_set_log_level(node.raw_node, log_level, &mut node_error)
        };

        if status != RnsEmbeddedStatus::Ok {
            return Err(format!("Set log level failed: {status:?}"));
        }

        Ok(())
    }

    /// Check if node is running
    pub fn is_running() -> bool {
        Self::global()
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|n| n.running))
            .unwrap_or(false)
    }

    /// Get the ABI version of the linked rns-embedded-ffi
    pub fn abi_version() -> u32 {
        unsafe { rns_embedded_v1_abi_version() }
    }

    /// Get capabilities of the linked rns-embedded-ffi
    pub fn capabilities() -> Result<Capabilities, String> {
        let mut caps = default_capabilities();
        let status = unsafe { rns_embedded_v1_get_capabilities(&mut caps) };
        if status != RnsEmbeddedStatus::Ok {
            return Err(format!("Get capabilities failed: {status:?}"));
        }
        Ok(Capabilities {
            abi_version: caps.abi_version,
            capability_bits: caps.capability_bits,
            max_event_payload_bytes: caps.max_event_payload_bytes,
            max_subscriptions: caps.max_subscriptions,
        })
    }
}

impl Drop for LxmfNode {
    fn drop(&mut self) {
        if let Some(sub) = self.subscription.take() {
            let mut err = default_node_error();
            unsafe { rns_embedded_v1_subscription_close(sub, &mut err) };
        }
        if self.running {
            let mut err = default_node_error();
            unsafe { rns_embedded_v1_node_stop(self.raw_node, &mut err) };
        }
        if !self.raw_node.is_null() {
            unsafe { rns_embedded_v1_node_free(self.raw_node) };
        }
    }
}

// --- Public types ---

#[derive(Debug, Clone)]
pub struct SendReceipt {
    pub operation_id: u64,
    pub accepted_bytes: usize,
    pub queued: bool,
}

#[derive(Debug, Clone)]
pub struct NodeStatus {
    pub running: bool,
    pub lifecycle: u32,
    pub epoch: u64,
    pub pending_outbound: usize,
    pub outbound_sent: u32,
    pub inbound_accepted: u32,
    pub announces_received: u32,
    pub lxmf_messages_received: u32,
}

#[derive(Debug, Clone)]
pub struct Capabilities {
    pub abi_version: u32,
    pub capability_bits: u64,
    pub max_event_payload_bytes: u32,
    pub max_subscriptions: u32,
}

// --- Internal helpers ---

fn map_event(event: &RnsEmbeddedV1NodeEvent, node: &mut LxmfNode) -> Option<LxmfEvent> {
    match event.kind {
        RnsEmbeddedV1EventKind::StatusChanged => {
            Some(LxmfEvent::StatusChanged {
                running: event.run_state == rns_embedded_ffi::RnsEmbeddedV1RunState::Running,
                lifecycle: event.lifecycle_state as u32,
            })
        }
        RnsEmbeddedV1EventKind::PacketReceived => {
            // The event payload is in the node's internal buffer
            // Frame kind distinguishes SINGLE vs GROUP
            if event.frame_kind == 0x01 {
                // GROUP destination = Solana tx relay
                Some(LxmfEvent::TxReceived {
                    data: vec![], // populated by native layer from packet buffer
                })
            } else {
                Some(LxmfEvent::PacketReceived {
                    source: [0u8; 16], // populated by native layer
                    data: vec![],
                })
            }
        }
        RnsEmbeddedV1EventKind::Log => {
            Some(LxmfEvent::Log {
                level: event.log_level as u32,
                message: format!("seq={} bytes={}", event.sequence, event.bytes),
            })
        }
        RnsEmbeddedV1EventKind::Error => {
            Some(LxmfEvent::Error {
                code: event.error_code as u32,
                message: format!("Node error code={:?}", event.error_code),
            })
        }
        RnsEmbeddedV1EventKind::PacketSent => None, // internal bookkeeping
        RnsEmbeddedV1EventKind::Extension => None,
    }
}

fn default_node_error() -> RnsEmbeddedV1NodeError {
    RnsEmbeddedV1NodeError {
        struct_size: std::mem::size_of::<RnsEmbeddedV1NodeError>(),
        struct_version: 1,
        code: RnsEmbeddedV1NodeErrorCode::Unknown,
        reserved: [0u8; 16],
    }
}

fn default_send_receipt() -> RnsEmbeddedV1SendReceipt {
    RnsEmbeddedV1SendReceipt {
        struct_size: std::mem::size_of::<RnsEmbeddedV1SendReceipt>(),
        struct_version: 1,
        operation_id: 0,
        epoch: 0,
        accepted_bytes: 0,
        queued: false,
        target_count: 0,
        reserved: [0u8; 24],
    }
}

fn default_node_status() -> RnsEmbeddedV1NodeStatus {
    RnsEmbeddedV1NodeStatus {
        struct_size: std::mem::size_of::<RnsEmbeddedV1NodeStatus>(),
        struct_version: 1,
        run_state: rns_embedded_ffi::RnsEmbeddedV1RunState::Stopped,
        epoch: 0,
        lifecycle_state: RnsEmbeddedLifecycleState::Boot,
        pending_outbound: 0,
        announces_queued: 0,
        outbound_sent: 0,
        outbound_deferred: 0,
        inbound_accepted: 0,
        inbound_rejected: 0,
        announces_received: 0,
        lxmf_messages_received: 0,
        log_level: RnsEmbeddedV1LogLevel::Info,
        reserved: [0u8; 24],
    }
}

fn default_poll_result() -> RnsEmbeddedV1PollResult {
    RnsEmbeddedV1PollResult {
        struct_size: std::mem::size_of::<RnsEmbeddedV1PollResult>(),
        struct_version: 1,
        kind: RnsEmbeddedV1PollResultKind::Timeout,
        next_event_id: 0,
        epoch: 0,
        reserved: [0u8; 24],
    }
}

fn default_node_event() -> RnsEmbeddedV1NodeEvent {
    RnsEmbeddedV1NodeEvent {
        struct_size: std::mem::size_of::<RnsEmbeddedV1NodeEvent>(),
        struct_version: 1,
        kind: RnsEmbeddedV1EventKind::StatusChanged,
        event_id: 0,
        epoch: 0,
        occurred_at_ms: 0,
        operation_id: 0,
        has_operation_id: false,
        run_state: rns_embedded_ffi::RnsEmbeddedV1RunState::Stopped,
        lifecycle_state: RnsEmbeddedLifecycleState::Boot,
        log_level: RnsEmbeddedV1LogLevel::Info,
        error_code: RnsEmbeddedV1NodeErrorCode::Unknown,
        frame_kind: 0,
        sequence: 0,
        bytes: 0,
        extension_id: 0,
        value0: 0,
        value1: 0,
        reserved: [0u8; 24],
    }
}

fn default_capabilities() -> RnsEmbeddedV1Capabilities {
    RnsEmbeddedV1Capabilities {
        struct_size: std::mem::size_of::<RnsEmbeddedV1Capabilities>(),
        struct_version: 1,
        abi_version: 0,
        capability_schema_version: 0,
        known_capability_bits: 0,
        compile_time_capability_bits: 0,
        capability_bits: 0,
        max_event_payload_bytes: 0,
        max_subscriptions: 0,
        max_blocking_timeout_ms: 0,
        driver_tick_target_ms: 0,
        driver_tick_max_ms: 0,
        reserved: [0u8; 24],
    }
}
