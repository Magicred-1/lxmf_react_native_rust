//! Reticulum TCP Bridge — connects to standard rnsd via HDLC-framed TCP
//!
//! Uses the legacy rns-embedded-ffi API (push_inbound_wire / take_outbound_wire / tick)
//! with proper HDLC framing so we speak the real Reticulum wire protocol.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::thread;
use std::time::{Duration, Instant};

use rns_embedded_ffi::{
    RnsEmbeddedNode, RnsEmbeddedNodeConfig, RnsEmbeddedNodeMode,
    RnsEmbeddedLinkState, RnsEmbeddedStatus, RnsEmbeddedLifecycleState,
    rns_embedded_node_new, rns_embedded_node_free, rns_embedded_node_tick,
    rns_embedded_node_push_inbound_wire, rns_embedded_node_take_outbound_wire,
    rns_embedded_node_set_link_state, rns_embedded_node_set_network_provisioned,
    rns_embedded_node_queue_message, rns_embedded_node_config_default,
    rns_embedded_node_get_lifecycle_state,
};

use crate::framing::{hdlc_encode, HdlcDeframer};
use crate::node::{IdentityKey, LxmfAddress, DestHash};

use log::{info, error, debug, warn};

/// Configuration for the bridge
pub struct BridgeConfig {
    pub identity: IdentityKey,
    pub lxmf_address: LxmfAddress,
    pub tcp_host: String,
    pub tcp_port: u16,
    pub announce_interval_ms: u64,
    pub ble_mtu_hint: u16,
}

/// The bridge wrapping a legacy RnsEmbeddedNode + TCP connection
pub struct ReticulumTcpBridge {
    node: *mut RnsEmbeddedNode,
    running: AtomicBool,
    tcp_stream: Mutex<Option<TcpStream>>,
}

// Raw pointer is managed safely through our Mutex
unsafe impl Send for ReticulumTcpBridge {}
unsafe impl Sync for ReticulumTcpBridge {}

impl ReticulumTcpBridge {
    /// Create and start the bridge. Connects to rnsd and starts the tick loop.
    pub fn start(config: BridgeConfig) -> Result<Arc<Self>, String> {
        info!("ReticulumBridge: connecting to {}:{}", config.tcp_host, config.tcp_port);

        // Connect to rnsd
        let addr = format!("{}:{}", config.tcp_host, config.tcp_port);
        let stream = TcpStream::connect(&addr)
            .map_err(|e| format!("TCP connect to {} failed: {}", addr, e))?;
        stream.set_nonblocking(true)
            .map_err(|e| format!("set_nonblocking failed: {}", e))?;
        stream.set_nodelay(true).ok();

        info!("ReticulumBridge: TCP connected to {}", addr);

        // Create legacy node with BLE mode (we handle TCP ourselves)
        let mut ffi_config = unsafe { rns_embedded_node_config_default() };
        ffi_config.store_identity = config.identity;
        ffi_config.lxmf_address = config.lxmf_address;
        ffi_config.node_mode = RnsEmbeddedNodeMode::BleOnly;
        ffi_config.announce_interval_ms = config.announce_interval_ms;
        ffi_config.ble_mtu_hint = config.ble_mtu_hint;

        let node = unsafe { rns_embedded_node_new(&ffi_config) };
        if node.is_null() {
            return Err("Failed to create legacy RnsEmbeddedNode".into());
        }

        // Mark link as up since we have TCP
        unsafe {
            rns_embedded_node_set_link_state(node, RnsEmbeddedLinkState::Up);
            rns_embedded_node_set_network_provisioned(node, true);
        }

        let bridge = Arc::new(ReticulumTcpBridge {
            node,
            running: AtomicBool::new(true),
            tcp_stream: Mutex::new(Some(stream)),
        });

        // Start the main loop in a background thread
        let bridge_clone = Arc::clone(&bridge);
        thread::Builder::new()
            .name("reticulum-bridge".into())
            .spawn(move || bridge_clone.run_loop())
            .map_err(|e| format!("Failed to spawn bridge thread: {}", e))?;

        info!("ReticulumBridge: started successfully");
        Ok(bridge)
    }

    /// Send a message through the bridge
    pub fn queue_message(&self, destination: &DestHash, body: &[u8]) -> Result<u32, String> {
        let mut seq: u32 = 0;
        let status = unsafe {
            rns_embedded_node_queue_message(
                self.node,
                destination.as_ptr(),
                body.as_ptr(),
                body.len(),
                &mut seq,
            )
        };
        if status != RnsEmbeddedStatus::Ok {
            return Err(format!("queue_message failed: {:?}", status));
        }
        Ok(seq)
    }

    /// Get the current lifecycle state
    pub fn lifecycle_state(&self) -> RnsEmbeddedLifecycleState {
        unsafe { rns_embedded_node_get_lifecycle_state(self.node) }
    }

    /// Check if bridge is running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Stop the bridge
    pub fn stop(&self) {
        info!("ReticulumBridge: stopping");
        self.running.store(false, Ordering::Relaxed);
        // Close TCP to unblock any reads
        if let Ok(mut guard) = self.tcp_stream.lock() {
            guard.take();
        }
    }

    /// Main tick + I/O loop
    fn run_loop(&self) {
        let mut deframer = HdlcDeframer::new();
        let mut read_buf = [0u8; 4096];
        let mut outbound_buf = [0u8; 2048];
        let tick_interval = Duration::from_millis(25); // match driver_tick_target_ms

        let start = Instant::now();

        while self.running.load(Ordering::Relaxed) {
            let now_ms = start.elapsed().as_millis() as u64;

            // 1. Tick the node state machine
            let tick_status = unsafe { rns_embedded_node_tick(self.node, now_ms) };
            if tick_status != RnsEmbeddedStatus::Ok {
                debug!("ReticulumBridge: tick returned {:?}", tick_status);
            }

            // 2. Read from TCP → deframe HDLC → push_inbound_wire
            if let Ok(guard) = self.tcp_stream.lock() {
                if let Some(ref stream) = *guard {
                    // Clone for read (TcpStream implements Clone as dup'd fd)
                    let mut reader = stream.try_clone().unwrap();
                    drop(guard);

                    match reader.read(&mut read_buf) {
                        Ok(0) => {
                            warn!("ReticulumBridge: TCP connection closed by peer");
                            self.running.store(false, Ordering::Relaxed);
                            break;
                        }
                        Ok(n) => {
                            let frames = deframer.feed(&read_buf[..n]);
                            for frame in frames {
                                debug!("ReticulumBridge: inbound frame {} bytes", frame.len());
                                let status = unsafe {
                                    rns_embedded_node_push_inbound_wire(
                                        self.node,
                                        frame.as_ptr(),
                                        frame.len(),
                                    )
                                };
                                if status != RnsEmbeddedStatus::Ok {
                                    debug!("ReticulumBridge: push_inbound_wire: {:?}", status);
                                }
                            }
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            // No data available, that's fine
                        }
                        Err(e) => {
                            error!("ReticulumBridge: TCP read error: {}", e);
                            self.running.store(false, Ordering::Relaxed);
                            break;
                        }
                    }
                } else {
                    break;
                }
            }

            // 3. take_outbound_wire → HDLC encode → write to TCP
            loop {
                let mut out_len: usize = 0;
                let status = unsafe {
                    rns_embedded_node_take_outbound_wire(
                        self.node,
                        outbound_buf.as_mut_ptr(),
                        outbound_buf.len(),
                        &mut out_len,
                    )
                };

                if status == RnsEmbeddedStatus::NotFound || out_len == 0 {
                    break; // No more outbound frames
                }

                if status != RnsEmbeddedStatus::Ok {
                    debug!("ReticulumBridge: take_outbound_wire: {:?}", status);
                    break;
                }

                let wire_data = &outbound_buf[..out_len];
                let hdlc_frame = hdlc_encode(wire_data);
                debug!("ReticulumBridge: outbound frame {} bytes -> {} HDLC bytes", out_len, hdlc_frame.len());

                if let Ok(guard) = self.tcp_stream.lock() {
                    if let Some(ref stream) = *guard {
                        let mut writer = stream.try_clone().unwrap();
                        drop(guard);
                        if let Err(e) = writer.write_all(&hdlc_frame) {
                            error!("ReticulumBridge: TCP write error: {}", e);
                            self.running.store(false, Ordering::Relaxed);
                            break;
                        }
                    }
                }
            }

            thread::sleep(tick_interval);
        }

        info!("ReticulumBridge: loop ended, cleaning up");
    }
}

impl Drop for ReticulumTcpBridge {
    fn drop(&mut self) {
        self.stop();
        if !self.node.is_null() {
            unsafe { rns_embedded_node_free(self.node) };
        }
    }
}
