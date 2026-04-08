//! LxmfNode — full Reticulum node using rns-transport
//!
//! Mode 0: BLE only (embedded FFI)
//! Mode 3: Standard Reticulum TCP (rns-transport with real protocol)
//!
//! The rns-transport mode creates a proper Reticulum node that speaks the
//! real wire protocol, generates identity, sends announces, and is visible
//! to all other nodes on the network.

use std::sync::{Arc, Mutex, OnceLock};
use std::collections::VecDeque;

use log::{info, warn};
use serde_json;

use crate::beacon::BeaconManager;
use crate::store::MessageStore;

use rns_transport::transport::Transport;

/// Destination hash: 16 bytes identifying a Reticulum destination
pub type DestHash = [u8; 16];

/// Identity key: 32 bytes for the node's cryptographic identity
pub type IdentityKey = [u8; 32];

/// LXMF address: 16 bytes
pub type LxmfAddress = [u8; 16];

/// Events emitted to the native layer (Swift/Kotlin) for forwarding to JS
#[derive(Debug, Clone)]
pub enum LxmfEvent {
    StatusChanged { running: bool, lifecycle: u32 },
    PacketReceived { source: DestHash, data: Vec<u8> },
    TxReceived { data: Vec<u8> },
    BeaconDiscovered { dest_hash: DestHash, app_data: Vec<u8> },
    MessageReceived { source: LxmfAddress, content: Vec<u8>, timestamp: u64 },
    AnnounceReceived { dest_hash: DestHash, app_data: Vec<u8>, hops: u8 },
    Log { level: u32, message: String },
    Error { code: u32, message: String },
}

pub type EventQueue = Arc<Mutex<VecDeque<LxmfEvent>>>;

/// Global singleton
static NODE: OnceLock<Mutex<Option<LxmfNode>>> = OnceLock::new();

/// Handle to the tokio runtime (one per process)
static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn get_runtime() -> &'static tokio::runtime::Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime")
    })
}

/// The main LXMF node — wraps either embedded FFI or full rns-transport
pub struct LxmfNode {
    /// Event queue polled by native layer
    pub events: EventQueue,
    /// Beacon manager
    pub beacon_mgr: BeaconManager,
    /// Message persistence
    pub store: Option<MessageStore>,
    /// Running state
    running: bool,
    /// Identity hex (for display)
    pub identity_hex: String,
    /// Address hex (for display)
    pub address_hex: String,
    /// The mode we started with
    mode: u32,
    /// Private identity bytes (64 bytes, persisted)
    identity_bytes: Option<Vec<u8>>,
    /// Reticulum transport handle (mode 3 only)
    transport: Option<Arc<tokio::sync::Mutex<Transport>>>,
}

// Access through Mutex
unsafe impl Send for LxmfNode {}

impl LxmfNode {
    pub fn global() -> &'static Mutex<Option<LxmfNode>> {
        NODE.get_or_init(|| Mutex::new(None))
    }

    /// Initialize — create the node shell. Does not start networking yet.
    pub fn init(db_path: Option<&str>) -> Result<(), String> {
        let store = db_path.map(|p| {
            MessageStore::open(p).map_err(|e| format!("SQLite open failed: {e}"))
        }).transpose()?;

        let node = LxmfNode {
            events: Arc::new(Mutex::new(VecDeque::with_capacity(256))),
            beacon_mgr: BeaconManager::new(),
            store,
            running: false,
            identity_hex: String::new(),
            address_hex: String::new(),
            mode: 0,
            identity_bytes: None,
            transport: None,
        };

        let mut guard = Self::global().lock().map_err(|e| e.to_string())?;
        *guard = Some(node);
        info!("LxmfNode: initialized");
        Ok(())
    }

    /// Start the node.
    /// mode 0: BLE only (embedded FFI)
    /// mode 3: Standard Reticulum TCP via rns-transport
    pub fn start(
        identity_hex: &str,
        address_hex: &str,
        mode: u32,
        announce_interval_ms: u64,
        _ble_mtu_hint: u16,
        tcp_host: Option<&str>,
        tcp_port: u16,
    ) -> Result<(), String> {
        info!("LxmfNode::start mode={} host={:?} port={}", mode, tcp_host, tcp_port);

        match mode {
            3 => Self::start_reticulum(identity_hex, address_hex, tcp_host, tcp_port, announce_interval_ms),
            0 => Self::start_ble(identity_hex, address_hex),
            _ => Err(format!("Unsupported mode: {}. Use 0 (BLE) or 3 (Reticulum TCP)", mode)),
        }
    }

    /// Start with full Reticulum transport (mode 3)
    fn start_reticulum(
        identity_hex: &str,
        address_hex: &str,
        tcp_host: Option<&str>,
        tcp_port: u16,
        announce_interval_ms: u64,
    ) -> Result<(), String> {
        use rns_transport::identity::PrivateIdentity;
        use rns_transport::transport::TransportConfig;
        use rns_transport::destination::DestinationName;
        use rns_transport::iface::tcp_client::TcpClient;

        let host = tcp_host.ok_or("Reticulum TCP mode requires a tcpHost")?;
        let addr = format!("{}:{}", host, tcp_port);

        // Create or restore identity
        let private_identity = if identity_hex.len() == 128 {
            // 64 bytes = full private key
            PrivateIdentity::new_from_hex_string(identity_hex)
                .map_err(|e| format!("Invalid identity hex: {:?}", e))?
        } else {
            // Generate new identity
            info!("LxmfNode: generating new identity");
            PrivateIdentity::new_from_rand(rand_core::OsRng)
        };

        let id_hex = private_identity.to_hex_string();
        let addr_hash = private_identity.address_hash();
        let addr_hex = hex::encode(addr_hash.as_slice());

        info!("LxmfNode: identity={} address={}", &id_hex[..16], addr_hex);

        // Store identity bytes for persistence
        let id_bytes = private_identity.to_private_key_bytes().to_vec();

        // Get event queue handle
        let events = {
            let guard = Self::global().lock().map_err(|e| e.to_string())?;
            let node = guard.as_ref().ok_or("Node not initialized")?;
            Arc::clone(&node.events)
        };

        let rt = get_runtime();

        // Set up transport synchronously so we can store the handle
        let (transport_arc, my_dest, mut data_rx, announce_rx) = rt.block_on(async {
            let config = TransportConfig::new("lxmf-mobile", &private_identity, true);
            let mut transport = Transport::new(config);

            // Add TCP interface to reach the network
            {
                let iface_mgr = transport.iface_manager();
                let mut mgr = iface_mgr.lock().await;
                info!("LxmfNode: connecting TCP to {}", addr);
                mgr.spawn(TcpClient::new(&addr), TcpClient::spawn);
            }

            // Register LXMF delivery destination
            let dest_name = DestinationName::new("lxmf", "delivery");
            let my_dest = transport.add_destination(private_identity.clone(), dest_name).await;

            // Give the TCP interface time to connect
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

            // Send initial announce
            info!("LxmfNode: sending announce");
            transport.send_announce(&my_dest, Some(b"lxmf-mobile")).await;

            let data_rx = transport.received_data_events();
            let announce_rx = transport.recv_announces().await;

            let arc = Arc::new(tokio::sync::Mutex::new(transport));
            (arc, my_dest, data_rx, announce_rx)
        });

        // Push status event
        if let Ok(mut eq) = events.lock() {
            eq.push_back(LxmfEvent::StatusChanged { running: true, lifecycle: 3 });
        }

        // Spawn announce receiver
        let events_ann = Arc::clone(&events);
        let mut announce_rx = announce_rx;
        rt.spawn(async move {
            loop {
                match announce_rx.recv().await {
                    Ok(event) => {
                        let dest = event.destination.lock().await;
                        let hash_bytes = dest.desc.address_hash;
                        let mut dh = [0u8; 16];
                        dh.copy_from_slice(hash_bytes.as_slice());
                        let app_data = event.app_data.as_slice().to_vec();
                        info!("LxmfNode: announce from {} ({} hops)", hex::encode(&dh), event.hops);
                        if let Ok(mut eq) = events_ann.lock() {
                            eq.push_back(LxmfEvent::AnnounceReceived {
                                dest_hash: dh,
                                app_data,
                                hops: event.hops,
                            });
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("LxmfNode: lagged {} announce events", n);
                    }
                    Err(_) => break,
                }
            }
        });

        // Spawn data receiver
        let events_data = Arc::clone(&events);
        rt.spawn(async move {
            loop {
                match data_rx.recv().await {
                    Ok(received) => {
                        let mut src = [0u8; 16];
                        src.copy_from_slice(received.destination.as_slice());
                        let data = received.data.as_slice().to_vec();
                        info!("LxmfNode: received {} bytes from {}", data.len(), hex::encode(&src));
                        if let Ok(mut eq) = events_data.lock() {
                            eq.push_back(LxmfEvent::MessageReceived {
                                source: src,
                                content: data,
                                timestamp: std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs(),
                            });
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("LxmfNode: lagged {} data events", n);
                    }
                    Err(_) => break,
                }
            }
        });

        // Spawn periodic re-announce
        let transport_reannounce = Arc::clone(&transport_arc);
        let interval_ms = if announce_interval_ms > 0 { announce_interval_ms } else { 300_000 };
        rt.spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(interval_ms)).await;
                info!("LxmfNode: periodic re-announce");
                transport_reannounce
                    .lock()
                    .await
                    .send_announce(&my_dest, Some(b"lxmf-mobile"))
                    .await;
            }
        });

        // Update node state
        let mut guard = Self::global().lock().map_err(|e| e.to_string())?;
        let node = guard.as_mut().ok_or("Node not initialized")?;
        node.running = true;
        node.mode = 3;
        node.identity_hex = id_hex;
        node.address_hex = addr_hex;
        node.identity_bytes = Some(id_bytes);
        node.transport = Some(transport_arc);

        info!("LxmfNode: Reticulum transport started");
        Ok(())
    }

    /// Send data to a destination identified by its 32-char hex address hash.
    /// The transport handles encryption using the destination's announced public key.
    pub fn send_to(dest_hex: &str, data: &[u8]) -> Result<(), String> {
        use rns_transport::hash::AddressHash;
        use rns_transport::packet::{Packet, PacketDataBuffer};
        use rns_transport::transport::SendPacketOutcome;

        let transport = {
            let guard = Self::global().lock().map_err(|e| e.to_string())?;
            let node = guard.as_ref().ok_or("Node not initialized")?;
            node.transport.clone().ok_or("Transport not started (mode 3 only)")?
        };

        let dest_bytes = hex::decode(dest_hex)
            .map_err(|e| format!("Invalid dest hex: {e}"))?;
        if dest_bytes.len() != 16 {
            return Err(format!("dest must be 16 bytes (32 hex chars), got {}", dest_bytes.len()));
        }
        let mut dest_arr = [0u8; 16];
        dest_arr.copy_from_slice(&dest_bytes);

        let packet = Packet {
            destination: AddressHash::new(dest_arr),
            data: PacketDataBuffer::new_from_slice(data),
            ..Default::default()
        };

        let outcome = get_runtime().block_on(async move {
            let transport = transport.lock().await;
            transport.send_packet_with_outcome(packet).await
        });

        match outcome {
            SendPacketOutcome::SentDirect | SendPacketOutcome::SentBroadcast => {
                info!("LxmfNode::send_to: packet dispatched for {} ({outcome:?})", dest_hex);
                Ok(())
            }
            SendPacketOutcome::DroppedMissingDestinationIdentity => {
                Err(format!("missing destination identity for /{dest_hex}/"))
            }
            SendPacketOutcome::DroppedNoRoute => {
                Err(format!("no route to /{dest_hex}/"))
            }
            SendPacketOutcome::DroppedCiphertextTooLarge => {
                Err("message payload too large after encryption".to_string())
            }
            SendPacketOutcome::DroppedEncryptFailed => {
                Err(format!("failed to encrypt packet for /{dest_hex}/"))
            }
        }
    }

    /// Start in BLE-only mode (mode 0) — uses embedded FFI
    fn start_ble(identity_hex: &str, address_hex: &str) -> Result<(), String> {
        let mut guard = Self::global().lock().map_err(|e| e.to_string())?;
        let node = guard.as_mut().ok_or("Node not initialized")?;

        if node.running {
            return Err("Node already running".into());
        }

        node.running = true;
        node.mode = 0;
        node.identity_hex = identity_hex.to_string();
        node.address_hex = address_hex.to_string();
        node.beacon_mgr.start_announce_schedule();

        info!("LxmfNode: BLE mode started");
        Ok(())
    }

    /// Stop the node
    pub fn stop() -> Result<(), String> {
        let mut guard = Self::global().lock().map_err(|e| e.to_string())?;
        let node = guard.as_mut().ok_or("Node not initialized")?;

        if !node.running {
            return Ok(());
        }

        // TODO: graceful transport shutdown
        node.running = false;
        node.beacon_mgr.stop();
        info!("LxmfNode: stopped");
        Ok(())
    }

    /// Check if running
    pub fn is_running() -> bool {
        Self::global()
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|n| n.running))
            .unwrap_or(false)
    }

    /// Get status as JSON
    pub fn get_status_json() -> Result<String, String> {
        let guard = Self::global().lock().map_err(|e| e.to_string())?;
        let node = guard.as_ref().ok_or("Node not initialized")?;

        let json = serde_json::json!({
            "running": node.running,
            "mode": node.mode,
            "identityHex": &node.identity_hex[..std::cmp::min(32, node.identity_hex.len())],
            "addressHex": &node.address_hex,
            "lifecycle": if node.running { 3 } else { 0 },
            "epoch": 0,
            "pendingOutbound": 0,
            "outboundSent": 0,
            "inboundAccepted": 0,
            "announcesReceived": 0,
            "lxmfMessagesReceived": 0,
        }).to_string();

        Ok(json)
    }

    /// Drain pending events
    pub fn drain_events() -> Vec<LxmfEvent> {
        let mut guard = match Self::global().lock() {
            Ok(g) => g,
            Err(_) => return vec![],
        };
        let node = match guard.as_mut() {
            Some(n) => n,
            None => return vec![],
        };

        let mut events = Vec::new();
        if let Ok(mut eq) = node.events.lock() {
            while let Some(ev) = eq.pop_front() {
                events.push(ev);
            }
        }

        for log_line in crate::log_bridge::drain_logs() {
            events.push(LxmfEvent::Log {
                level: log_line.level,
                message: log_line.message,
            });
        }

        events.extend(node.beacon_mgr.drain_events());
        events
    }

    /// Get the node's identity hex (full 128-char private key hex for persistence)
    pub fn get_identity_hex() -> Option<String> {
        Self::global()
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|n| n.identity_hex.clone()))
    }

    /// Get the node's address hex
    pub fn get_address_hex() -> Option<String> {
        Self::global()
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|n| n.address_hex.clone()))
    }

    pub fn abi_version() -> u32 {
        2 // v2 = rns-transport based
    }
}
