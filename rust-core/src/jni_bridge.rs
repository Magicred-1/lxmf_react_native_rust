//! JNI bridge for Android — maps Kotlin native method declarations to LxmfNode API
//!
//! Each function matches a Kotlin `external fun` in expo.modules.lxmf.LxmfModule.
//! Type conversions: JString <-> Rust String, jbyteArray <-> Vec<u8>.

use jni::JNIEnv;
use jni::objects::{JByteArray, JClass, JString};
use jni::sys::{jint, jlong, jboolean, jshort};

use crate::node::{LxmfNode, DestHash, IdentityKey, LxmfAddress};

const JNI_TRUE: jboolean = 1;
const JNI_FALSE: jboolean = 0;

// --- Lifecycle ---

#[no_mangle]
pub extern "C" fn Java_expo_modules_lxmf_LxmfModule_nativeInit(
    mut env: JNIEnv,
    _class: JClass,
    db_path: JString,
) -> jint {
    let path: Option<String> = if db_path.is_null() {
        None
    } else {
        env.get_string(&db_path).ok().map(|s| s.into())
    };

    match LxmfNode::init(path.as_deref()) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

#[no_mangle]
pub extern "C" fn Java_expo_modules_lxmf_LxmfModule_nativeStart(
    mut env: JNIEnv,
    _class: JClass,
    identity: JByteArray,
    lxmf_address: JByteArray,
    mode: jint,
    announce_interval_ms: jlong,
    ble_mtu_hint: jshort,
    tcp_host: JString,
    tcp_port: jshort,
) -> jint {
    let identity_bytes = match env.convert_byte_array(&identity) {
        Ok(b) => b,
        Err(_) => return -1,
    };
    let address_bytes = match env.convert_byte_array(&lxmf_address) {
        Ok(b) => b,
        Err(_) => return -1,
    };

    if identity_bytes.len() != 32 || address_bytes.len() != 16 {
        return -1;
    }

    let mut id_arr: IdentityKey = [0u8; 32];
    id_arr.copy_from_slice(&identity_bytes);
    let mut addr_arr: LxmfAddress = [0u8; 16];
    addr_arr.copy_from_slice(&address_bytes);

    let host: Option<String> = if tcp_host.is_null() {
        None
    } else {
        env.get_string(&tcp_host).ok().map(|s| s.into())
    };

    match LxmfNode::start(
        &id_arr,
        &addr_arr,
        mode as u32,
        announce_interval_ms as u64,
        ble_mtu_hint as u16,
        host.as_deref(),
        tcp_port as u16,
    ) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

#[no_mangle]
pub extern "C" fn Java_expo_modules_lxmf_LxmfModule_nativeStop(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    match LxmfNode::stop() {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

#[no_mangle]
pub extern "C" fn Java_expo_modules_lxmf_LxmfModule_nativeIsRunning(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    if LxmfNode::is_running() { JNI_TRUE } else { JNI_FALSE }
}

// --- Messaging ---

#[no_mangle]
pub extern "C" fn Java_expo_modules_lxmf_LxmfModule_nativeSend(
    mut env: JNIEnv,
    _class: JClass,
    dest: JByteArray,
    body: JByteArray,
) -> jlong {
    let dest_bytes = match env.convert_byte_array(&dest) {
        Ok(b) if b.len() == 16 => b,
        _ => return -1,
    };
    let body_bytes = match env.convert_byte_array(&body) {
        Ok(b) => b,
        Err(_) => return -1,
    };

    let mut dest_arr: DestHash = [0u8; 16];
    dest_arr.copy_from_slice(&dest_bytes);

    match LxmfNode::send(&dest_arr, &body_bytes) {
        Ok(receipt) => receipt.operation_id as jlong,
        Err(_) => -1,
    }
}

#[no_mangle]
pub extern "C" fn Java_expo_modules_lxmf_LxmfModule_nativeBroadcast(
    mut env: JNIEnv,
    _class: JClass,
    dests: JByteArray,
    dest_count: jint,
    body: JByteArray,
) -> jlong {
    let dests_bytes = match env.convert_byte_array(&dests) {
        Ok(b) => b,
        Err(_) => return -1,
    };
    let body_bytes = match env.convert_byte_array(&body) {
        Ok(b) => b,
        Err(_) => return -1,
    };

    let destinations: Vec<DestHash> = dests_bytes.chunks_exact(16).map(|chunk| {
        let mut arr = [0u8; 16];
        arr.copy_from_slice(chunk);
        arr
    }).collect();

    if destinations.len() != dest_count as usize {
        return -1;
    }

    match LxmfNode::broadcast(&destinations, &body_bytes) {
        Ok(receipt) => receipt.operation_id as jlong,
        Err(_) => -1,
    }
}

// --- Event polling ---

#[no_mangle]
pub extern "C" fn Java_expo_modules_lxmf_LxmfModule_nativePollEvents(
    mut env: JNIEnv,
    _class: JClass,
    timeout_ms: jlong,
) -> JByteArray {
    let events = LxmfNode::poll_events(timeout_ms as u64);
    let json = crate::ffi::events_to_json_internal(&events);
    let bytes = json.as_bytes();

    match env.byte_array_from_slice(bytes) {
        Ok(arr) => arr,
        Err(_) => JByteArray::default(),
    }
}

// --- Status ---

#[no_mangle]
pub extern "C" fn Java_expo_modules_lxmf_LxmfModule_nativeGetStatus(
    mut env: JNIEnv,
    _class: JClass,
) -> JByteArray {
    let status = match LxmfNode::get_status() {
        Ok(s) => s,
        Err(_) => return JByteArray::default(),
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

    match env.byte_array_from_slice(json.as_bytes()) {
        Ok(arr) => arr,
        Err(_) => JByteArray::default(),
    }
}

// --- Beacons ---

#[no_mangle]
pub extern "C" fn Java_expo_modules_lxmf_LxmfModule_nativeGetBeacons(
    mut env: JNIEnv,
    _class: JClass,
) -> JByteArray {
    let guard = match LxmfNode::global().lock() {
        Ok(g) => g,
        Err(_) => return JByteArray::default(),
    };
    let node = match guard.as_ref() {
        Some(n) => n,
        None => return JByteArray::default(),
    };

    let json = node.beacon_mgr.beacons_json();
    match env.byte_array_from_slice(json.as_bytes()) {
        Ok(arr) => arr,
        Err(_) => JByteArray::default(),
    }
}

#[no_mangle]
pub extern "C" fn Java_expo_modules_lxmf_LxmfModule_nativeOnAnnounce(
    mut env: JNIEnv,
    _class: JClass,
    dest_hash: JByteArray,
    app_data: JByteArray,
) -> jint {
    let dest_bytes = match env.convert_byte_array(&dest_hash) {
        Ok(b) if b.len() == 16 => b,
        _ => return -1,
    };
    let app_data_bytes = match env.convert_byte_array(&app_data) {
        Ok(b) => b,
        Err(_) => return -1,
    };

    let mut dest_arr: DestHash = [0u8; 16];
    dest_arr.copy_from_slice(&dest_bytes);

    let mut guard = match LxmfNode::global().lock() {
        Ok(g) => g,
        Err(_) => return -1,
    };
    let node = match guard.as_mut() {
        Some(n) => n,
        None => return -1,
    };

    node.beacon_mgr.on_announce_received(dest_arr, &app_data_bytes);
    0
}

// --- Log level ---

#[no_mangle]
pub extern "C" fn Java_expo_modules_lxmf_LxmfModule_nativeSetLogLevel(
    _env: JNIEnv,
    _class: JClass,
    level: jint,
) -> jint {
    match LxmfNode::set_log_level(level as u32) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

// --- Messages ---

#[no_mangle]
pub extern "C" fn Java_expo_modules_lxmf_LxmfModule_nativeFetchMessages(
    mut env: JNIEnv,
    _class: JClass,
    limit: jint,
) -> JByteArray {
    let guard = match LxmfNode::global().lock() {
        Ok(g) => g,
        Err(_) => return JByteArray::default(),
    };
    let node = match guard.as_ref() {
        Some(n) => n,
        None => return JByteArray::default(),
    };

    let json = match &node.store {
        Some(store) => store.fetch_messages(limit as u32).unwrap_or_else(|_| "[]".into()),
        None => "[]".into(),
    };

    match env.byte_array_from_slice(json.as_bytes()) {
        Ok(arr) => arr,
        Err(_) => JByteArray::default(),
    }
}

// --- ABI ---

#[no_mangle]
pub extern "C" fn Java_expo_modules_lxmf_LxmfModule_nativeAbiVersion(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    LxmfNode::abi_version() as jint
}
