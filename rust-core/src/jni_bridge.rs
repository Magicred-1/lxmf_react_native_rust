//! JNI bridge for Android — maps Kotlin native method declarations to LxmfNode API

use jni::JNIEnv;
use jni::objects::{JClass, JString};
use jni::sys::{jint, jlong, jboolean, jshort, jstring};
use log::error;
use serde_json;

use crate::node::LxmfNode;

const JNI_TRUE: jboolean = 1;
const JNI_FALSE: jboolean = 0;

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
    crate::log_bridge::init_logger(log::LevelFilter::Debug);

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
        Err(e) => { throw_err(&mut env, &format!("bad identity: {}", e)); return -1; }
    };
    let addr_str: String = match env.get_string(&lxmf_address_hex) {
        Ok(s) => s.into(),
        Err(e) => { throw_err(&mut env, &format!("bad address: {}", e)); return -1; }
    };

    let host: Option<String> = if tcp_host.is_null() {
        None
    } else {
        env.get_string(&tcp_host).ok().map(|s| s.into())
    };

    error!("LxmfModule: starting node mode={} host={:?} port={}", mode, host, tcp_port);

    match LxmfNode::start(
        &id_str,
        &addr_str,
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
    mut env: JNIEnv,
    _class: JClass,
) -> jint {
    match LxmfNode::stop() {
        Ok(()) => 0,
        Err(e) => { throw_err(&mut env, &format!("stop failed: {}", e)); -1 }
    }
}

#[no_mangle]
pub extern "C" fn Java_expo_modules_lxmf_LxmfModule_nativeIsRunning(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    if LxmfNode::is_running() { JNI_TRUE } else { JNI_FALSE }
}

// --- Events ---

#[no_mangle]
pub extern "C" fn Java_expo_modules_lxmf_LxmfModule_nativePollEvents(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    let events = LxmfNode::drain_events();
    if events.is_empty() {
        return std::ptr::null_mut();
    }
    let json = crate::ffi::events_to_json_internal(&events);
    match env.new_string(&json) {
        Ok(s) => s.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

// --- Status ---

#[no_mangle]
pub extern "C" fn Java_expo_modules_lxmf_LxmfModule_nativeGetStatus(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    match LxmfNode::get_status_json() {
        Ok(json) => match env.new_string(&json) {
            Ok(s) => s.into_raw(),
            Err(_) => std::ptr::null_mut(),
        },
        Err(_) => std::ptr::null_mut(),
    }
}

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

// --- Messaging ---

#[no_mangle]
pub extern "C" fn Java_expo_modules_lxmf_LxmfModule_nativeSend(
    mut env: JNIEnv,
    _class: JClass,
    dest_hex: JString,
    body_base64: JString,
) -> jlong {
    let dest: String = match env.get_string(&dest_hex) {
        Ok(s) => s.into(),
        Err(_) => { throw_err(&mut env, "nativeSend: invalid dest_hex string"); return -1; }
    };
    let body_b64: String = match env.get_string(&body_base64) {
        Ok(s) => s.into(),
        Err(_) => { throw_err(&mut env, "nativeSend: invalid body_base64 string"); return -1; }
    };

    use base64::Engine as _;
    let data = match base64::engine::general_purpose::STANDARD.decode(&body_b64) {
        Ok(d) => d,
        Err(e) => {
            throw_err(&mut env, &format!("nativeSend: base64 decode failed: {e}"));
            return -1;
        }
    };

    match LxmfNode::send_to(&dest, &data) {
        Ok(()) => 0,
        Err(e) => {
            error!("LxmfModule: send_to failed: {}", e);
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn Java_expo_modules_lxmf_LxmfModule_nativeBroadcast(
    mut env: JNIEnv,
    _class: JClass,
    dests_json: JString,
    body_base64: JString,
) -> jlong {
    // Broadcast = send to each destination in the JSON array
    let dests_str: String = match env.get_string(&dests_json) {
        Ok(s) => s.into(),
        Err(_) => { throw_err(&mut env, "nativeBroadcast: invalid dests_json string"); return -1; }
    };
    let body_b64: String = match env.get_string(&body_base64) {
        Ok(s) => s.into(),
        Err(_) => { throw_err(&mut env, "nativeBroadcast: invalid body_base64 string"); return -1; }
    };

    use base64::Engine as _;
    let data = match base64::engine::general_purpose::STANDARD.decode(&body_b64) {
        Ok(d) => d,
        Err(e) => {
            throw_err(&mut env, &format!("nativeBroadcast: base64 decode failed: {e}"));
            return -1;
        }
    };

    let dests: Vec<String> = match serde_json::from_str(&dests_str) {
        Ok(v) => v,
        Err(e) => {
            throw_err(&mut env, &format!("nativeBroadcast: JSON parse failed: {e}"));
            return -1;
        }
    };

    let mut sent: i64 = 0;
    for dest in &dests {
        match LxmfNode::send_to(dest, &data) {
            Ok(()) => sent += 1,
            Err(e) => error!("LxmfModule: broadcast send_to {} failed: {}", dest, e),
        }
    }
    sent
}

// --- Config ---

#[no_mangle]
pub extern "C" fn Java_expo_modules_lxmf_LxmfModule_nativeSetLogLevel(
    _env: JNIEnv,
    _class: JClass,
    level: jint,
) -> jint {
    crate::log_bridge::set_max_level_from_u32(level as u32);
    0
}

#[no_mangle]
pub extern "C" fn Java_expo_modules_lxmf_LxmfModule_nativeAbiVersion(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    LxmfNode::abi_version() as jint
}
