//! C FFI exports — called by iOS (Swift) and directly by Android when JNI is not preferred
//!
//! All functions are `#[no_mangle] extern "C"` with pointer+length patterns.
//! The native layer (Swift/Kotlin) calls these, and they delegate to LxmfNode.

use std::ffi::CStr;
use std::os::raw::c_char;
use std::slice;

use crate::node::{LxmfNode, DestHash};
use crate::framing::{hdlc_encode, kiss_encode};

pub const STATUS_OK: i32 = 0;
pub const STATUS_ERR: i32 = -1;
pub const STATUS_NOT_INIT: i32 = -2;

// --- Lifecycle ---

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

#[no_mangle]
pub unsafe extern "C" fn lxmf_start(
    identity_hex: *const c_char,
    address_hex: *const c_char,
    mode: u32,
    announce_interval_ms: u64,
    ble_mtu_hint: u16,
    tcp_host: *const c_char,
    tcp_port: u16,
) -> i32 {
    let id = if identity_hex.is_null() { "" } else {
        match CStr::from_ptr(identity_hex).to_str() { Ok(s) => s, Err(_) => return STATUS_ERR }
    };
    let addr = if address_hex.is_null() { "" } else {
        match CStr::from_ptr(address_hex).to_str() { Ok(s) => s, Err(_) => return STATUS_ERR }
    };
    let host = if tcp_host.is_null() { None } else {
        CStr::from_ptr(tcp_host).to_str().ok()
    };

    match LxmfNode::start(id, addr, mode, announce_interval_ms, ble_mtu_hint, host, tcp_port) {
        Ok(()) => STATUS_OK,
        Err(_) => STATUS_ERR,
    }
}

#[no_mangle]
pub unsafe extern "C" fn lxmf_stop() -> i32 {
    match LxmfNode::stop() {
        Ok(()) => STATUS_OK,
        Err(_) => STATUS_ERR,
    }
}

#[no_mangle]
pub unsafe extern "C" fn lxmf_is_running() -> i32 {
    if LxmfNode::is_running() { 1 } else { 0 }
}

// --- Status ---

#[no_mangle]
pub unsafe extern "C" fn lxmf_get_status(out_buf: *mut u8, out_capacity: usize) -> i32 {
    let json = match LxmfNode::get_status_json() {
        Ok(s) => s,
        Err(_) => return STATUS_ERR,
    };
    let bytes = json.as_bytes();
    if bytes.len() > out_capacity { return STATUS_ERR; }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_buf, bytes.len());
    bytes.len() as i32
}

// --- Events ---

#[no_mangle]
pub unsafe extern "C" fn lxmf_poll_events(
    _timeout_ms: u64,
    out_buf: *mut u8,
    out_capacity: usize,
) -> i32 {
    let events = LxmfNode::drain_events();
    if events.is_empty() { return 0; }

    let json = events_to_json(&events);
    let bytes = json.as_bytes();
    if bytes.len() > out_capacity { return STATUS_ERR; }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_buf, bytes.len());
    bytes.len() as i32
}

// --- Beacons ---

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
    if bytes.len() > out_capacity { return STATUS_ERR; }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_buf, bytes.len());
    bytes.len() as i32
}

#[no_mangle]
pub unsafe extern "C" fn lxmf_on_announce(
    dest_hash_ptr: *const u8,
    app_data_ptr: *const u8,
    app_data_len: usize,
) -> i32 {
    if dest_hash_ptr.is_null() || app_data_ptr.is_null() { return STATUS_ERR; }

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

// --- Messages ---

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
        Some(store) => store.fetch_messages(limit).unwrap_or_else(|_| "[]".into()),
        None => "[]".into(),
    };
    let bytes = json.as_bytes();
    if bytes.len() > out_capacity { return STATUS_ERR; }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_buf, bytes.len());
    bytes.len() as i32
}

// --- Config ---

#[no_mangle]
pub unsafe extern "C" fn lxmf_set_log_level(level: u32) -> i32 {
    crate::log_bridge::set_max_level_from_u32(level);
    STATUS_OK
}

#[no_mangle]
pub unsafe extern "C" fn lxmf_abi_version() -> u32 { LxmfNode::abi_version() }

// --- Framing helpers ---

#[no_mangle]
pub unsafe extern "C" fn lxmf_hdlc_encode(
    data_ptr: *const u8, data_len: usize, out_ptr: *mut u8, out_capacity: usize,
) -> i32 {
    if data_ptr.is_null() || out_ptr.is_null() { return STATUS_ERR; }
    let encoded = hdlc_encode(slice::from_raw_parts(data_ptr, data_len));
    if encoded.len() > out_capacity { return STATUS_ERR; }
    std::ptr::copy_nonoverlapping(encoded.as_ptr(), out_ptr, encoded.len());
    encoded.len() as i32
}

#[no_mangle]
pub unsafe extern "C" fn lxmf_kiss_encode(
    data_ptr: *const u8, data_len: usize, out_ptr: *mut u8, out_capacity: usize,
) -> i32 {
    if data_ptr.is_null() || out_ptr.is_null() { return STATUS_ERR; }
    let encoded = kiss_encode(slice::from_raw_parts(data_ptr, data_len));
    if encoded.len() > out_capacity { return STATUS_ERR; }
    std::ptr::copy_nonoverlapping(encoded.as_ptr(), out_ptr, encoded.len());
    encoded.len() as i32
}

// --- Internal ---

pub fn events_to_json_internal(events: &[crate::node::LxmfEvent]) -> String {
    events_to_json(events)
}

fn events_to_json(events: &[crate::node::LxmfEvent]) -> String {
    use crate::node::LxmfEvent;

    let arr: Vec<serde_json::Value> = events.iter().map(|e| match e {
        LxmfEvent::StatusChanged { running, lifecycle } => serde_json::json!({
            "type": "statusChanged", "running": running, "lifecycle": lifecycle,
        }),
        LxmfEvent::PacketReceived { source, data } => serde_json::json!({
            "type": "packetReceived", "source": hex::encode(source), "data": hex::encode(data),
        }),
        LxmfEvent::TxReceived { data } => serde_json::json!({
            "type": "txReceived", "data": hex::encode(data),
        }),
        LxmfEvent::BeaconDiscovered { dest_hash, app_data } => serde_json::json!({
            "type": "beaconDiscovered", "destHash": hex::encode(dest_hash),
            "appData": String::from_utf8_lossy(app_data).to_string(),
        }),
        LxmfEvent::MessageReceived { source, content, timestamp } => serde_json::json!({
            "type": "messageReceived", "source": hex::encode(source),
            "content": hex::encode(content), "timestamp": timestamp,
        }),
        LxmfEvent::AnnounceReceived { dest_hash, app_data, hops } => serde_json::json!({
            "type": "announceReceived", "destHash": hex::encode(dest_hash),
            "appData": String::from_utf8_lossy(app_data).to_string(), "hops": hops,
        }),
        LxmfEvent::Log { level, message } => serde_json::json!({
            "type": "log", "level": level, "message": message,
        }),
        LxmfEvent::Error { code, message } => serde_json::json!({
            "type": "error", "code": code, "message": message,
        }),
    }).collect();

    serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into())
}
