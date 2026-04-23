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
use crate::ble_iface::BleInterface;
use crate::nus_iface::NusInterface;
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
    /// Runtime counters
    pub outbound_sent: u64,
    pub inbound_accepted: u64,
    pub announces_received: u64,
    pub messages_received: u64,
}

// Access through Mutex
unsafe impl Send for LxmfNode {}

impl LxmfNode {
    pub fn global() -> &'static Mutex<Option<LxmfNode>> {
        NODE.get_or_init(|| Mutex::new(None))
    }

    /// Initialize — create the node shell. Does not start networking yet.
    pub fn init(db_path: Option<&str>) -> Result<(), String> {
        // Install the log bridge so Rust info!/warn!/error! logs flow to the
        // native event queue and appear in the UI's Debug Logs section.
        crate::log_bridge::init_logger(log::LevelFilter::Debug);

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
            outbound_sent: 0,
            inbound_accepted: 0,
            announces_received: 0,
            messages_received: 0,
        };

        // Clear stale BLE peer list from any previous session in this process
        crate::ble_iface::clear_ble_peers();

        let mut guard = Self::global().lock().map_err(|e| e.to_string())?;
        *guard = Some(node);
        info!("LxmfNode: initialized");
        Ok(())
    }

    /// Start the node.
    /// mode 0: BLE only (embedded FFI)
    /// mode 3: Standard Reticulum TCP via rns-transport
    ///
    /// `interfaces_json` is a JSON array of `{"host": "...", "port": 1234}` objects.
    /// At least one entry is required for mode 3.
    pub fn start(
        identity_hex: &str,
        address_hex: &str,
        mode: u32,
        announce_interval_ms: u64,
        _ble_mtu_hint: u16,
        interfaces_json: &str,
        display_name: &str,
    ) -> Result<(), String> {
        info!("LxmfNode::start mode={} interfaces={} name={}", mode, interfaces_json, display_name);

        match mode {
            3 => {
                let interfaces = parse_interfaces_json(interfaces_json)?;
                Self::start_reticulum(identity_hex, address_hex, &interfaces, announce_interval_ms, display_name)
            }
            0 => Self::start_ble(identity_hex, address_hex, display_name),
            _ => Err(format!("Unsupported mode: {}. Use 0 (BLE) or 3 (Reticulum TCP)", mode)),
        }
    }

    /// Start with full Reticulum transport (mode 3)
    fn start_reticulum(
        identity_hex: &str,
        address_hex: &str,
        interfaces: &[(String, u16)],
        announce_interval_ms: u64,
        display_name: &str,
    ) -> Result<(), String> {
        use rns_transport::identity::PrivateIdentity;
        use rns_transport::transport::TransportConfig;
        use rns_transport::destination::DestinationName;
        use rns_transport::iface::tcp_client::TcpClient;

        if interfaces.is_empty() {
            return Err("Reticulum TCP mode requires at least one interface".into());
        }

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
        // addr_hex is set later from my_dest.desc.address_hash (LXMF delivery destination hash)

        info!("LxmfNode: identity={}", &id_hex[..16]);

        // Store identity bytes for persistence
        let id_bytes = private_identity.to_private_key_bytes().to_vec();

        // Get event queue handle
        let events = {
            let guard = Self::global().lock().map_err(|e| e.to_string())?;
            let node = guard.as_ref().ok_or("Node not initialized")?;
            Arc::clone(&node.events)
        };

        let rt = get_runtime();

        // Clamp display name to 32 bytes, fall back to "lxmf-mobile"
        let name_bytes: Vec<u8> = if display_name.is_empty() {
            b"lxmf-mobile".to_vec()
        } else {
            display_name.as_bytes()[..display_name.len().min(32)].to_vec()
        };

        // Set up transport synchronously so we can store the handle
        let name_bytes_init = name_bytes.clone();
        let (transport_arc, my_dest, mut data_rx, mut resource_rx, announce_rx, lxmf_addr_hex) = rt.block_on(async move {
            let config = TransportConfig::new("lxmf-mobile", &private_identity, true);
            let mut transport = Transport::new(config);

            // Add all TCP interfaces
            {
                let iface_mgr = transport.iface_manager();
                let mut mgr = iface_mgr.lock().await;
                for (host, port) in interfaces {
                    let addr = format!("{}:{}", host, port);
                    info!("LxmfNode: connecting TCP to {}", addr);
                    mgr.spawn(TcpClient::new(&addr), TcpClient::spawn);
                }
            }

            // Register LXMF delivery destination
            let dest_name = DestinationName::new("lxmf", "delivery");
            let my_dest = transport.add_destination(private_identity.clone(), dest_name).await;

            // Extract the LXMF delivery destination hash while we still have async context.
            // This is Hash(name_hash + identity_address_hash) — NOT the raw identity hash.
            // Peers must send to this address, and we embed it as source in outgoing messages.
            let lxmf_addr_hex = {
                let d = my_dest.lock().await;
                hex::encode(d.desc.address_hash.as_slice())
            };

            // Give the TCP interface time to connect
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

            // Send initial announce with display name as app_data
            info!("LxmfNode: sending announce as {}", lxmf_addr_hex);
            transport.send_announce(&my_dest, Some(name_bytes_init.as_slice())).await;

            let data_rx = transport.in_link_events();
            let resource_rx = transport.resource_events();
            let announce_rx = transport.recv_announces().await;

            let arc = Arc::new(tokio::sync::Mutex::new(transport));
            (arc, my_dest, data_rx, resource_rx, announce_rx, lxmf_addr_hex)
        });

        let addr_hex = lxmf_addr_hex;

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

        // Spawn data receiver — uses in_link_events() to avoid echo of own outbound packets
        let events_data = Arc::clone(&events);
        rt.spawn(async move {
            use rns_transport::destination::link::LinkEvent;
            loop {
                match data_rx.recv().await {
                    Ok(event) => {
                        if let LinkEvent::Data(payload) = event.event {
                            let mut src = [0u8; 16];
                            src.copy_from_slice(event.address_hash.as_slice());
                            let data = payload.as_slice().to_vec();
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
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("LxmfNode: lagged {} link events", n);
                    }
                    Err(_) => break,
                }
            }
        });

        // Spawn resource receiver — handles large messages (>MTU) delivered via resource transfer
        let events_res = Arc::clone(&events);
        rt.spawn(async move {
            use rns_transport::resource::ResourceEventKind;
            loop {
                match resource_rx.recv().await {
                    Ok(event) => {
                        if let ResourceEventKind::Complete(complete) = event.kind {
                            let mut src = [0u8; 16];
                            src.copy_from_slice(event.link_id.as_slice());
                            let data = complete.data;
                            info!("LxmfNode: resource complete {} bytes from {}", data.len(), hex::encode(&src));
                            if let Ok(mut eq) = events_res.lock() {
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
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("LxmfNode: lagged {} resource events", n);
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
                    .send_announce(&my_dest, Some(name_bytes.as_slice()))
                    .await;
            }
        });

        info!("LxmfNode: LXMF delivery address = {}", addr_hex);

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

    /// Send an LXMF message to a destination.
    ///
    /// Encodes content as a proper LXMF wire message:
    ///   [16B dest_hash][16B source_hash][64B Ed25519 sig][msgpack payload]
    /// where msgpack payload = [timestamp: f64, title: bytes, content: bytes, fields: {}]
    ///
    /// The Reticulum transport then encrypts and routes the packet.
    pub fn send_to(dest_hex: &str, content: &[u8]) -> Result<(), String> {
        use rns_transport::hash::AddressHash;
        use rns_transport::identity::PrivateIdentity;
        use rns_transport::packet::{Packet, PacketDataBuffer};
        use rns_transport::transport::SendPacketOutcome;

        let (transport, identity_bytes, source_hash_bytes) = {
            let guard = Self::global().lock().map_err(|e| e.to_string())?;
            let node = guard.as_ref().ok_or("Node not initialized")?;
            let transport = node.transport.clone().ok_or("Transport not started (mode 3 only)")?;
            let id_bytes = node.identity_bytes.clone().ok_or("No identity available")?;
            let addr_hex = node.address_hex.clone();
            let src = hex::decode(&addr_hex).map_err(|e| format!("Bad address hex: {e}"))?;
            (transport, id_bytes, src)
        };

        let dest_bytes = hex::decode(dest_hex)
            .map_err(|e| format!("Invalid dest hex: {e}"))?;
        if dest_bytes.len() != 16 {
            return Err(format!("dest must be 16 bytes (32 hex chars), got {}", dest_bytes.len()));
        }
        if source_hash_bytes.len() != 16 {
            return Err(format!("source address must be 16 bytes, got {}", source_hash_bytes.len()));
        }

        let mut dest_arr = [0u8; 16];
        dest_arr.copy_from_slice(&dest_bytes);

        // Rebuild identity for signing
        let private_identity = PrivateIdentity::from_private_key_bytes(&identity_bytes)
            .map_err(|e| format!("Failed to restore identity: {:?}", e))?;

        // Encode LXMF msgpack payload: [timestamp: f64, title: bin, content: bin, fields: fixmap{}]
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        let msgpack = encode_lxmf_msgpack(timestamp, b"", content);

        // Build: dest_hash(16) + source_hash(16) + signature(64) + msgpack
        // Signature covers: dest_hash + source_hash + msgpack
        let mut sign_data = Vec::with_capacity(16 + 16 + msgpack.len());
        sign_data.extend_from_slice(&dest_arr);
        sign_data.extend_from_slice(&source_hash_bytes);
        sign_data.extend_from_slice(&msgpack);
        let signature = private_identity.sign(&sign_data).to_bytes();

        let mut lxmf_payload = Vec::with_capacity(16 + 16 + 64 + msgpack.len());
        lxmf_payload.extend_from_slice(&dest_arr);
        lxmf_payload.extend_from_slice(&source_hash_bytes);
        lxmf_payload.extend_from_slice(&signature);
        lxmf_payload.extend_from_slice(&msgpack);

        info!("LxmfNode::send_to: dest={} payload={}B", dest_hex, lxmf_payload.len());

        let packet = Packet {
            destination: AddressHash::new(dest_arr),
            data: PacketDataBuffer::new_from_slice(&lxmf_payload),
            ..Default::default()
        };

        let outcome = get_runtime().block_on(async move {
            let transport = transport.lock().await;
            transport.send_packet_with_outcome(packet).await
        });

        match outcome {
            SendPacketOutcome::SentDirect | SendPacketOutcome::SentBroadcast => {
                info!("LxmfNode::send_to: dispatched ({outcome:?})");
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

    /// Start in BLE-only mode (mode 0).
    ///
    /// Sets up a full rns-transport instance with BleInterface instead of TcpClient.
    /// The Kotlin BleManager must be started separately (it owns hardware access).
    /// Call `nativeBleConnected` / `nativeBleDisconnected` / `nativeBleReceive` from Kotlin
    /// as BLE peers connect and send data.
    fn start_ble(identity_hex: &str, _address_hex: &str, display_name: &str) -> Result<(), String> {
        use rns_transport::identity::PrivateIdentity;
        use rns_transport::transport::TransportConfig;
        use rns_transport::destination::DestinationName;

        if Self::is_running() {
            return Err("Node already running".into());
        }

        // Create or restore identity.
        let private_identity = if identity_hex.len() == 128 {
            PrivateIdentity::new_from_hex_string(identity_hex)
                .map_err(|e| format!("Invalid identity hex: {:?}", e))?
        } else {
            info!("LxmfNode BLE: generating new identity");
            PrivateIdentity::new_from_rand(rand_core::OsRng)
        };

        let id_hex = private_identity.to_hex_string();
        let id_bytes = private_identity.to_private_key_bytes().to_vec();

        let events = {
            let guard = Self::global().lock().map_err(|e| e.to_string())?;
            let node = guard.as_ref().ok_or("Node not initialized")?;
            Arc::clone(&node.events)
        };

        let rt = get_runtime();
        let display_name = display_name.to_owned();

        let (transport_arc, my_dest, mut data_rx, announce_rx, addr_hex) =
            rt.block_on(async move {
                let config = TransportConfig::new("lxmf-ble", &private_identity, true);
                let mut transport = Transport::new(config);

                // Register BLE interface — phone-to-phone mesh (HDLC + segmentation).
                // Register NUS interface — RNode BLE (KISS framing).
                {
                    let iface_mgr = transport.iface_manager();
                    let mut mgr = iface_mgr.lock().await;
                    mgr.spawn(BleInterface::new(), BleInterface::spawn);
                    mgr.spawn(NusInterface::new(), NusInterface::spawn);
                }

                // Register LXMF delivery destination.
                let dest_name = DestinationName::new("lxmf", "delivery");
                let my_dest = transport.add_destination(private_identity.clone(), dest_name).await;

                let addr_hex = {
                    let d = my_dest.lock().await;
                    hex::encode(d.desc.address_hash.as_slice())
                };

                // Brief pause to let BleInterface start its poll loop.
                tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

                // Send initial announce (broadcast to any connected BLE peers).
                info!("LxmfNode BLE: sending announce as {}", addr_hex);
                let ble_name: Vec<u8> = if display_name.is_empty() {
                    b"lxmf-mobile".to_vec()
                } else {
                    display_name.as_bytes()[..display_name.len().min(32)].to_vec()
                };
                transport.send_announce(&my_dest, Some(ble_name.as_slice())).await;

                let data_rx = transport.received_data_events();
                let announce_rx = transport.recv_announces().await;
                let arc = Arc::new(tokio::sync::Mutex::new(transport));
                (arc, my_dest, data_rx, announce_rx, addr_hex)
            });

        info!("LxmfNode BLE: LXMF delivery address = {}", addr_hex);

        // Push status event.
        if let Ok(mut eq) = events.lock() {
            eq.push_back(LxmfEvent::StatusChanged { running: true, lifecycle: 0 });
        }

        // Spawn announce receiver.
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
                        info!("LxmfNode BLE: announce from {} ({} hops)", hex::encode(&dh), event.hops);
                        if let Ok(mut eq) = events_ann.lock() {
                            eq.push_back(LxmfEvent::AnnounceReceived {
                                dest_hash: dh,
                                app_data,
                                hops: event.hops,
                            });
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("LxmfNode BLE: lagged {} announce events", n);
                    }
                    Err(_) => break,
                }
            }
        });

        // Spawn data receiver.
        let events_data = Arc::clone(&events);
        rt.spawn(async move {
            loop {
                match data_rx.recv().await {
                    Ok(received) => {
                        let mut src = [0u8; 16];
                        src.copy_from_slice(received.destination.as_slice());
                        let data = received.data.as_slice().to_vec();
                        info!("LxmfNode BLE: received {} bytes", data.len());
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
                        warn!("LxmfNode BLE: lagged {} data events", n);
                    }
                    Err(_) => break,
                }
            }
        });

        // Update node state.
        let mut guard = Self::global().lock().map_err(|e| e.to_string())?;
        let node = guard.as_mut().ok_or("Node not initialized")?;
        node.running = true;
        node.mode = 0;
        node.identity_hex = id_hex;
        node.address_hex = addr_hex;
        node.identity_bytes = Some(id_bytes);
        node.transport = Some(transport_arc);

        info!("LxmfNode: BLE transport started");
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
            "outboundSent": node.outbound_sent,
            "inboundAccepted": node.inbound_accepted,
            "announcesReceived": node.announces_received,
            "lxmfMessagesReceived": node.messages_received,
            "blePeerCount": crate::ble_iface::ble_peer_count() as u32,
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

        // Update counters based on drained events
        for ev in &events {
            match ev {
                LxmfEvent::AnnounceReceived { .. } => node.announces_received += 1,
                LxmfEvent::MessageReceived { .. } => node.messages_received += 1,
                _ => {}
            }
        }

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

/// Encode LXMF msgpack payload: fixarray(4) [timestamp:f64, title:bin, content:bin, fields:fixmap{}]
fn encode_lxmf_msgpack(timestamp: f64, title: &[u8], content: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + 9 + 2 + title.len() + 2 + content.len() + 1);

    // fixarray with 4 elements: 0x94
    buf.push(0x94);

    // timestamp as float64 (0xcb)
    buf.push(0xcb);
    buf.extend_from_slice(&timestamp.to_bits().to_be_bytes());

    // title as bin8
    buf.push(0xc4);
    buf.push(title.len() as u8);
    buf.extend_from_slice(title);

    // content as bin8 (or bin16 if > 255 bytes)
    if content.len() <= 255 {
        buf.push(0xc4);
        buf.push(content.len() as u8);
    } else {
        buf.push(0xc5);
        buf.push((content.len() >> 8) as u8);
        buf.push((content.len() & 0xff) as u8);
    }
    buf.extend_from_slice(content);

    // fields: fixmap with 0 entries (0x80)
    buf.push(0x80);

    buf
}

/// Parse a JSON interfaces array: `[{"host":"...","port":1234}, ...]`
fn parse_interfaces_json(json: &str) -> Result<Vec<(String, u16)>, String> {
    let arr: Vec<serde_json::Value> = serde_json::from_str(json)
        .map_err(|e| format!("Invalid interfaces JSON: {}", e))?;
    let mut out = Vec::with_capacity(arr.len());
    for (i, v) in arr.iter().enumerate() {
        let host = v["host"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("Interface {}: missing or empty \"host\"", i))?
            .to_string();
        let port = v["port"]
            .as_u64()
            .filter(|&p| p > 0 && p <= 65535)
            .ok_or_else(|| format!("Interface {}: invalid \"port\"", i))? as u16;
        out.push((host, port));
    }
    Ok(out)
}
