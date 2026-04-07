//! JNI bridge for Android — maps Kotlin native method declarations to LxmfNode API
//!
//! Each function matches a Kotlin `external fun` in expo.modules.lxmf.LxmfModule.
//! Type conversions: JString <-> Rust String, jbyteArray <-> Vec<u8>.

use jni::JNIEnv;
use jni::objects::{JByteArray, JClass, JString, JObject};
use jni::sys::{jint, jlong, jboolean, jshort, jstring};
use log::error;

use crate::node::{LxmfNode, DestHash, IdentityKey, LxmfAddress};

const JNI_TRUE: jboolean = 1;
const JNI_FALSE: jboolean = 0;

/// Log an error and throw a Java RuntimeException so JS gets the real message.
fn throw_err(env: &mut JNIEnv, msg: &str) {
    error!("LxmfModule JNI: {}", msg);
    let _ = env.throw_new("java/lang/RuntimeException", msg);
}

// --- Lifecycle ---

#[no_mangle]
pub extern "C" fn Java_expo_modules_lxmf_LxmfModule_nativeInit(
    mut env: JNIEnv,
    _class: JClass,
    db_path: JString,
) -> jint {
    // Initialize android logger on first call
    #[cfg(target_os = "android")]
    {
        let _ = android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Debug)
                .with_tag("LxmfRust"),
        );
    }

    let path: Option<String> = if db_path.is_null() {
        None
    } else {
        env.get_string(&db_path).ok().map(|s| s.into())
    };

    match LxmfNode::init(path.as_deref()) {
        Ok(()) => 0,
        Err(e) => {
            throw_err(&mut env, &format!("init failed: {}", e));
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn Java_expo_modules_lxmf_LxmfModule_nativeStart(
    mut env: JNIEnv,
    _class: JClass,
    identity_hex: JString,
    lxmf_address_hex: JString,
    mode: jint,
    announce_interval_ms: jlong,
    ble_mtu_hint: jshort,
    tcp_host: JString,
    tcp_port: jshort,
) -> jint {
    let id_str: String = match env.get_string(&identity_hex) {
        Ok(s) => s.into(),
        Err(e) => {
            throw_err(&mut env, &format!("bad identity string: {}", e));
            return -1;
        }
    };
    let addr_str: String = match env.get_string(&lxmf_address_hex) {
        Ok(s) => s.into(),
        Err(e) => {
            throw_err(&mut env, &format!("bad address string: {}", e));
            return -1;
        }
    };

    let identity_bytes = match hex::decode(&id_str) {
        Ok(b) if b.len() == 32 => b,
        Ok(b) => {
            throw_err(&mut env, &format!("identity hex wrong length: {} bytes (need 32)", b.len()));
            return -1;
        }
        Err(e) => {
            throw_err(&mut env, &format!("identity hex decode failed: {}", e));
            return -1;
        }
    };
    let address_bytes = match hex::decode(&addr_str) {
        Ok(b) if b.len() == 16 => b,
        Ok(b) => {
            throw_err(&mut env, &format!("address hex wrong length: {} bytes (need 16)", b.len()));
            return -1;
        }
        Err(e) => {
            throw_err(&mut env, &format!("address hex decode failed: {}", e));
            return -1;
        }
    };

    let mut id_arr: IdentityKey = [0u8; 32];
    id_arr.copy_from_slice(&identity_bytes);
    let mut addr_arr: LxmfAddress = [0u8; 16];
    addr_arr.copy_from_slice(&address_bytes);

    let host: Option<String> = if tcp_host.is_null() {
        None
    } else {
        env.get_string(&tcp_host).ok().map(|s| s.into())
    };

    error!("LxmfModule: starting node mode={} host={:?} port={}", mode, host, tcp_port);

    match LxmfNode::start(
        &id_arr,
        &addr_arr,
        mode as u32,
        announce_interval_ms as u64,
        ble_mtu_hint as u16,
        host.as_deref(),
        tcp_port as u16,
    ) {
        Ok(()) => {
            error!("LxmfModule: node started successfully");
            0
        }
        Err(e) => {
            throw_err(&mut env, &format!("start failed: {}", e));
            -1
        }
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
    dest_hex: JString,
    body_base64: JString,
) -> jlong {
    let dest_str: String = match env.get_string(&dest_hex) {
        Ok(s) => s.into(),
        Err(_) => return -1,
    };
    let body_str: String = match env.get_string(&body_base64) {
        Ok(s) => s.into(),
        Err(_) => return -1,
    };

    let dest_bytes = match hex::decode(&dest_str) {
        Ok(b) if b.len() == 16 => b,
        _ => return -1,
    };
    let body_bytes = body_str.as_bytes().to_vec();

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
    dests_json: JString,
    body_base64: JString,
) -> jlong {
    let dests_str: String = match env.get_string(&dests_json) {
        Ok(s) => s.into(),
        Err(_) => return -1,
    };
    let body_str: String = match env.get_string(&body_base64) {
        Ok(s) => s.into(),
        Err(_) => return -1,
    };

    let hex_list: Vec<String> = match serde_json::from_str(&dests_str) {
        Ok(v) => v,
        Err(_) => return -1,
    };

    let mut destinations: Vec<DestHash> = Vec::with_capacity(hex_list.len());
    for h in &hex_list {
        match hex::decode(h) {
            Ok(b) if b.len() == 16 => {
                let mut arr = [0u8; 16];
                arr.copy_from_slice(&b);
                destinations.push(arr);
            }
            _ => return -1,
        }
    }

    let body_bytes = body_str.as_bytes().to_vec();

    match LxmfNode::broadcast(&destinations, &body_bytes) {
        Ok(receipt) => receipt.operation_id as jlong,
        Err(_) => -1,
    }
}

// --- Event polling ---

#[no_mangle]
pub extern "C" fn Java_expo_modules_lxmf_LxmfModule_nativePollEvents<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    timeout_ms: jlong,
) -> JByteArray<'local> {
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
) -> jstring {
    let status = match LxmfNode::get_status() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
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

    match env.new_string(&json) {
        Ok(s) => s.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

// --- Beacons ---

#[no_mangle]
pub extern "C" fn Java_expo_modules_lxmf_LxmfModule_nativeGetBeacons(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    let guard = match LxmfNode::global().lock() {
        Ok(g) => g,
        Err(_) => return std::ptr::null_mut(),
    };
    let node = match guard.as_ref() {
        Some(n) => n,
        None => return std::ptr::null_mut(),
    };

    let json = node.beacon_mgr.beacons_json();
    match env.new_string(&json) {
        Ok(s) => s.into_raw(),
        Err(_) => std::ptr::null_mut(),
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
) -> jstring {
    let guard = match LxmfNode::global().lock() {
        Ok(g) => g,
        Err(_) => return std::ptr::null_mut(),
    };
    let node = match guard.as_ref() {
        Some(n) => n,
        None => return std::ptr::null_mut(),
    };

    let json = match &node.store {
        Some(store) => store.fetch_messages(limit as u32).unwrap_or_else(|_| "[]".into()),
        None => "[]".into(),
    };

    match env.new_string(&json) {
        Ok(s) => s.into_raw(),
        Err(_) => std::ptr::null_mut(),
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
