//! C FFI exports — called by iOS (Swift) and directly by Android when JNI is not preferred
//!
//! All functions are `#[no_mangle] extern "C"` with pointer+length patterns.
//! The native layer (Swift/Kotlin) calls these, and they delegate to LxmfNode.

use std::ffi::CStr;
use std::os::raw::c_char;
use std::slice;

use crate::node::{LxmfNode, DestHash, IdentityKey, LxmfAddress};
use crate::framing::{hdlc_encode, kiss_encode};

/// Status codes returned to native layer
pub const STATUS_OK: i32 = 0;
pub const STATUS_ERR: i32 = -1;
pub const STATUS_NOT_INIT: i32 = -2;
pub const STATUS_NOT_RUNNING: i32 = -3;

// --- Lifecycle ---

/// Initialize the LXMF node. Call once at app startup.
/// `db_path`: null-terminated C string path for SQLite, or null to skip persistence.
#[no_mangle]
pub unsafe extern "C" fn lxmf_init(db_path: *const c_char) -> i32 {
    let path = if db_path.is_null() {
        None
    } else {
        CStr::from_ptr(db_path).to_str().ok()
    };

    match LxmfNode::init(path) {
        Ok(()) => STATUS_OK,
        Err(_) => STATUS_ERR,
    }
}

/// Start the LXMF node with the given configuration.
#[no_mangle]
pub unsafe extern "C" fn lxmf_start(
    identity_ptr: *const u8,
    lxmf_address_ptr: *const u8,
    mode: u32,
    announce_interval_ms: u64,
    ble_mtu_hint: u16,
    tcp_host: *const c_char,
    tcp_port: u16,
) -> i32 {
    if identity_ptr.is_null() || lxmf_address_ptr.is_null() {
        return STATUS_ERR;
    }

    let identity: IdentityKey = {
        let mut arr = [0u8; 32];
        arr.copy_from_slice(slice::from_raw_parts(identity_ptr, 32));
        arr
    };

    let lxmf_address: LxmfAddress = {
        let mut arr = [0u8; 16];
        arr.copy_from_slice(slice::from_raw_parts(lxmf_address_ptr, 16));
        arr
    };

    let host = if tcp_host.is_null() {
        None
    } else {
        CStr::from_ptr(tcp_host).to_str().ok()
    };

    match LxmfNode::start(&identity, &lxmf_address, mode, announce_interval_ms, ble_mtu_hint, host, tcp_port) {
        Ok(()) => STATUS_OK,
        Err(_) => STATUS_ERR,
    }
}

/// Stop the LXMF node.
#[no_mangle]
pub unsafe extern "C" fn lxmf_stop() -> i32 {
    match LxmfNode::stop() {
        Ok(()) => STATUS_OK,
        Err(_) => STATUS_ERR,
    }
}

/// Check if the node is running. Returns 1 if running, 0 if not.
#[no_mangle]
pub unsafe extern "C" fn lxmf_is_running() -> i32 {
    if LxmfNode::is_running() { 1 } else { 0 }
}

// --- Messaging ---

/// Send a message to a specific 16-byte destination hash.
/// Returns operation_id on success (>= 0), or negative error code.
#[no_mangle]
pub unsafe extern "C" fn lxmf_send(
    dest_ptr: *const u8,
    body_ptr: *const u8,
    body_len: usize,
) -> i64 {
    if dest_ptr.is_null() || body_ptr.is_null() {
        return STATUS_ERR as i64;
    }

    let mut dest: DestHash = [0u8; 16];
    dest.copy_from_slice(slice::from_raw_parts(dest_ptr, 16));
    let body = slice::from_raw_parts(body_ptr, body_len);

    match LxmfNode::send(&dest, body) {
        Ok(receipt) => receipt.operation_id as i64,
        Err(_) => STATUS_ERR as i64,
    }
}

/// Broadcast a message to multiple destinations.
/// `dests_ptr` points to N * 16 bytes (concatenated destination hashes).
#[no_mangle]
pub unsafe extern "C" fn lxmf_broadcast(
    dests_ptr: *const u8,
    dest_count: usize,
    body_ptr: *const u8,
    body_len: usize,
) -> i64 {
    if dests_ptr.is_null() || body_ptr.is_null() {
        return STATUS_ERR as i64;
    }

    let raw = slice::from_raw_parts(dests_ptr, dest_count * 16);
    let destinations: Vec<DestHash> = raw.chunks_exact(16).map(|chunk| {
        let mut arr = [0u8; 16];
        arr.copy_from_slice(chunk);
        arr
    }).collect();

    let body = slice::from_raw_parts(body_ptr, body_len);

    match LxmfNode::broadcast(&destinations, body) {
        Ok(receipt) => receipt.operation_id as i64,
        Err(_) => STATUS_ERR as i64,
    }
}

// --- Event polling ---

/// Poll for events. Writes serialized JSON into `out_buf`.
/// Returns the number of bytes written, or negative on error.
/// The native layer should call this on a timer (e.g., every 50-80ms).
#[no_mangle]
pub unsafe extern "C" fn lxmf_poll_events(
    timeout_ms: u64,
    out_buf: *mut u8,
    out_capacity: usize,
) -> i32 {
    let events = LxmfNode::poll_events(timeout_ms);
    if events.is_empty() {
        return 0;
    }

    let json = events_to_json(&events);
    let bytes = json.as_bytes();

    if bytes.len() > out_capacity {
        return STATUS_ERR; // buffer too small
    }

    std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_buf, bytes.len());
    bytes.len() as i32
}

// --- Status ---

/// Get node status as JSON. Writes into `out_buf`.
/// Returns bytes written or negative error.
#[no_mangle]
pub unsafe extern "C" fn lxmf_get_status(out_buf: *mut u8, out_capacity: usize) -> i32 {
    let status = match LxmfNode::get_status() {
        Ok(s) => s,
        Err(_) => return STATUS_ERR,
    };

    let json = serde_json::json!({
        "running": status.running,
        "lifecycle": status.lifecycle,
        "epoch": status.epoch,
        "pendingOutbound": status.pending_outbound,
        "outboundSent": status.outbound_sent,
        "inboundAccepted": status.inbound_accepted,
        "announcesReceived": status.announces_received,
        "lxmfMessagesReceived": status.lxmf_messages_received,
    }).to_string();

    let bytes = json.as_bytes();
    if bytes.len() > out_capacity {
        return STATUS_ERR;
    }

    std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_buf, bytes.len());
    bytes.len() as i32
}

// --- Beacon management ---

/// Get beacon pool state as JSON.
#[no_mangle]
pub unsafe extern "C" fn lxmf_get_beacons(out_buf: *mut u8, out_capacity: usize) -> i32 {
    let guard = match LxmfNode::global().lock() {
        Ok(g) => g,
        Err(_) => return STATUS_ERR,
    };
    let node = match guard.as_ref() {
        Some(n) => n,
        None => return STATUS_NOT_INIT,
    };

    let json = node.beacon_mgr.beacons_json();
    let bytes = json.as_bytes();

    if bytes.len() > out_capacity {
        return STATUS_ERR;
    }

    std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_buf, bytes.len());
    bytes.len() as i32
}

/// Notify that a beacon announce was received (called from native BLE/announce handler).
#[no_mangle]
pub unsafe extern "C" fn lxmf_on_announce(
    dest_hash_ptr: *const u8,
    app_data_ptr: *const u8,
    app_data_len: usize,
) -> i32 {
    if dest_hash_ptr.is_null() || app_data_ptr.is_null() {
        return STATUS_ERR;
    }

    let mut dest_hash: DestHash = [0u8; 16];
    dest_hash.copy_from_slice(slice::from_raw_parts(dest_hash_ptr, 16));
    let app_data = slice::from_raw_parts(app_data_ptr, app_data_len);

    let mut guard = match LxmfNode::global().lock() {
        Ok(g) => g,
        Err(_) => return STATUS_ERR,
    };
    let node = match guard.as_mut() {
        Some(n) => n,
        None => return STATUS_NOT_INIT,
    };

    node.beacon_mgr.on_announce_received(dest_hash, app_data);
    STATUS_OK
}

// --- Log level ---

#[no_mangle]
pub unsafe extern "C" fn lxmf_set_log_level(level: u32) -> i32 {
    match LxmfNode::set_log_level(level) {
        Ok(()) => STATUS_OK,
        Err(_) => STATUS_ERR,
    }
}

// --- ABI info ---

#[no_mangle]
pub unsafe extern "C" fn lxmf_abi_version() -> u32 {
    LxmfNode::abi_version()
}

// --- Framing helpers (for native BLE layer) ---

/// HDLC-encode a payload for BLE transmission.
#[no_mangle]
pub unsafe extern "C" fn lxmf_hdlc_encode(
    data_ptr: *const u8,
    data_len: usize,
    out_ptr: *mut u8,
    out_capacity: usize,
) -> i32 {
    if data_ptr.is_null() || out_ptr.is_null() {
        return STATUS_ERR;
    }

    let data = slice::from_raw_parts(data_ptr, data_len);
    let encoded = hdlc_encode(data);

    if encoded.len() > out_capacity {
        return STATUS_ERR;
    }

    std::ptr::copy_nonoverlapping(encoded.as_ptr(), out_ptr, encoded.len());
    encoded.len() as i32
}

/// KISS-encode a payload for RNode/LoRa transmission.
#[no_mangle]
pub unsafe extern "C" fn lxmf_kiss_encode(
    data_ptr: *const u8,
    data_len: usize,
    out_ptr: *mut u8,
    out_capacity: usize,
) -> i32 {
    if data_ptr.is_null() || out_ptr.is_null() {
        return STATUS_ERR;
    }

    let data = slice::from_raw_parts(data_ptr, data_len);
    let encoded = kiss_encode(data);

    if encoded.len() > out_capacity {
        return STATUS_ERR;
    }

    std::ptr::copy_nonoverlapping(encoded.as_ptr(), out_ptr, encoded.len());
    encoded.len() as i32
}

// --- Message persistence ---

/// Fetch recent messages as JSON.
#[no_mangle]
pub unsafe extern "C" fn lxmf_fetch_messages(
    limit: u32,
    out_buf: *mut u8,
    out_capacity: usize,
) -> i32 {
    let guard = match LxmfNode::global().lock() {
        Ok(g) => g,
        Err(_) => return STATUS_ERR,
    };
    let node = match guard.as_ref() {
        Some(n) => n,
        None => return STATUS_NOT_INIT,
    };

    let json = match &node.store {
        Some(store) => match store.fetch_messages(limit) {
            Ok(j) => j,
            Err(_) => return STATUS_ERR,
        },
        None => "[]".to_string(),
    };

    let bytes = json.as_bytes();
    if bytes.len() > out_capacity {
        return STATUS_ERR;
    }

    std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_buf, bytes.len());
    bytes.len() as i32
}

// --- Internal helpers ---

/// Serialize events to JSON (also used by JNI bridge)
pub fn events_to_json_internal(events: &[crate::node::LxmfEvent]) -> String {
    events_to_json(events)
}

fn events_to_json(events: &[crate::node::LxmfEvent]) -> String {
    use crate::node::LxmfEvent;

    let arr: Vec<serde_json::Value> = events.iter().map(|e| match e {
        LxmfEvent::StatusChanged { running, lifecycle } => serde_json::json!({
            "type": "statusChanged",
            "running": running,
            "lifecycle": lifecycle,
        }),
        LxmfEvent::PacketReceived { source, data } => serde_json::json!({
            "type": "packetReceived",
            "source": hex::encode(source),
            "data": hex::encode(data),
        }),
        LxmfEvent::TxReceived { data } => serde_json::json!({
            "type": "txReceived",
            "data": hex::encode(data),
        }),
        LxmfEvent::BeaconDiscovered { dest_hash, app_data } => serde_json::json!({
            "type": "beaconDiscovered",
            "destHash": hex::encode(dest_hash),
            "appData": String::from_utf8_lossy(app_data).to_string(),
        }),
        LxmfEvent::MessageReceived { source, content, timestamp } => serde_json::json!({
            "type": "messageReceived",
            "source": hex::encode(source),
            "content": hex::encode(content),
            "timestamp": timestamp,
        }),
        LxmfEvent::Log { level, message } => serde_json::json!({
            "type": "log",
            "level": level,
            "message": message,
        }),
        LxmfEvent::Error { code, message } => serde_json::json!({
            "type": "error",
            "code": code,
            "message": message,
        }),
    }).collect();

    serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into())
}
