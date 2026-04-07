# FFI Wiring: rns-embedded-ffi → Expo Native Modules

## Overview

This document explains how the Rust FFI layer connects to iOS/Android Expo native modules.

## Architecture Stack

```
┌─────────────────────┐
│  React Native App   │
│  (TypeScript)       │
└──────────┬──────────┘
           │ events: onPacketReceived, etc.
           │ methods: send(), start(), etc.
┌──────────▼──────────────────────────┐
│  Expo Module Bridge                 │
│ ┌────────────────┐  ┌─────────────┐ │
│ │ iOS (Swift)    │  │ Android(Kt) │ │
│ │ LxmfModule     │  │ LxmfModule  │ │
│ └────────┬───────┘  └──────┬──────┘ │
└─────────┼────────────────────┼───────┘
          │                    │
          │ C FFI calls        │ JNI calls
          │                    │
┌─────────▼────────────────────▼───────┐
│  Rust FFI (ffi.rs)                   │
│ lxmf_init()                          │
│ lxmf_start()                         │
│ lxmf_send()                          │
│ lxmf_poll_events()                   │
│ ... 20+ exported functions           │
└─────────────┬──────────────────────┘
              │ Rust function calls
┌─────────────▼──────────────────────┐
│  Rust Wrapper (node.rs)            │
│ LxmfNode::init()                   │
│ LxmfNode::start()                  │
│ LxmfNode::send()                   │
│ LxmfNode::poll_events()            │
└─────────────┬──────────────────────┘
              │ Creates/manages
┌─────────────▼──────────────────────┐
│  rns-embedded-ffi v1                │
│ (FreeTAKTeam/LXMF-rs)              │
│                                    │
│ Manages:                           │
│ - Reticulum node lifecycle        │
│ - BLE, LoRa, TCP/UDP transports   │
│ - Announce/link management        │
│ - X25519+AES-256-GCM crypto      │
└────────────────────────────────────┘
```

## Data Flow: Send Message

```
JS: lxmf.send("deadbeef...", base64Body)
    │
    ├─► useLxmf.ts: send() callback
    │    │
    │    └─► LxmfModule.send(destHex, bodyBase64)
    │
    ├─ iOS: LxmfModule.swift
    │    │
    │    └─► AsyncFunction("send") { destHex, bodyBase64 ->
    │         │
    │         └─► self.callJsonFfi { buf, cap in
    │              lxmf_send(buf.baseAddress, buf.count)
    │             }
    │
    ├─ Android: LxmfModule.kt
    │    │
    │    └─► AsyncFunction("send") { destHex, bodyBase64 ->
    │         │
    │         └─► nativeSend(destHex, bodyBase64)
    │              │
    │              └─► JNI call: Java_expo_modules_lxmf_LxmfModule_nativeSend
    │                   │
    │                   └─► Rust: jni_bridge.rs:
    │                        Java_expo_modules_lxmf_LxmfModule_nativeSend
    │
    ├─ Rust FFI: ffi.rs
    │    │
    │    └─► pub unsafe extern "C" fn lxmf_send(...)
    │         │
    │         └─► LxmfNode::send(dest, body)
    │
    ├─ Rust Node: node.rs
    │    │
    │    └─► pub fn send(...) -> Result<SendReceipt>
    │         │
    │         └─► unsafe { rns_embedded_v1_node_send(...) }
    │              │
    │              └─► rns-embedded-ffi: node sends via BLE/mesh
    │
    └─ Receipt returned: opId
       │
       ├─ iOS: return opId as Double
       │
       ├─ Android: return opId as Long → converted to Double in Kotlin
       │
       └─ JS: Promise resolves with opId
```

## Event Flow: Receive Message

```
rns-embedded-ffi: Packet received on mesh
    │
    └─► Rust: node.rs poll_events()
         │
         └─► LxmfEvent emitted to queue
              │
              ├─ iOS: LxmfModule.swift drainEvents() timer (80ms)
              │        │
              │        └─► lxmf_poll_events() → JSON string
              │             │
              │             └─► JSONSerialization.jsonObject
              │                  │
              │                  └─► sendEvent("onPacketReceived", event)
              │
              └─ Android: LxmfModule.kt poll() loop
                       │
                       └─► nativePolLEvents() JNI call
                            │
                            └─► jni_bridge.rs: reads event queue
                                 │
                                 └─► sendEvent("onPacketReceived", event)
    │
    └─ JS: useLxmf.ts event listener
         │
         └─► addListener('onPacketReceived', (event) => {
              eventBufferRef.current.push(event)
             })
         │
         └─► Event polled every 100ms
              │
              └─► setEvents([...buffer])
                   │
                   └─► React component renders
```

## C FFI Layer Details (ffi.rs)

### Functions Exported

```rust
// Lifecycle (returns i32 status code)
pub extern "C" fn lxmf_init(db_path: *const c_char) -> i32
pub extern "C" fn lxmf_start(
    identity_ptr: *const u8,
    lxmf_address_ptr: *const u8,
    mode: u32,
    announce_interval_ms: u64,
    ble_mtu_hint: u16,
    tcp_host: *const c_char,
    tcp_port: u16,
) -> i32
pub extern "C" fn lxmf_stop() -> i32
pub extern "C" fn lxmf_is_running() -> i32

// Messaging (returns i64 operation_id or error)
pub extern "C" fn lxmf_send(
    dest_ptr: *const u8,          // 16-byte destination hash
    body_ptr: *const u8,          // message body
    body_len: usize,
) -> i64

pub extern "C" fn lxmf_broadcast(
    dests_ptr: *const u8,         // N * 16 bytes
    dest_count: usize,
    body_ptr: *const u8,
    body_len: usize,
) -> i64

// Event polling (returns bytes written to buffer)
pub extern "C" fn lxmf_poll_events(
    timeout_ms: u64,
    out_buf: *mut u8,             // JSON output
    out_capacity: usize,
) -> i32

// Status queries (returns bytes written)
pub extern "C" fn lxmf_get_status(out_buf: *mut u8, out_capacity: usize) -> i32
pub extern "C" fn lxmf_get_beacons(out_buf: *mut u8, out_capacity: usize) -> i32
pub extern "C" fn lxmf_fetch_messages(limit: u32, out_buf: *mut u8, out_capacity: usize) -> i32

// Utilities
pub extern "C" fn lxmf_set_log_level(level: u32) -> i32
pub extern "C" fn lxmf_abi_version() -> u32

// Announce handler (called from BLE manager when beacon received)
pub extern "C" fn lxmf_on_announce(
    dest_hash_ptr: *const u8,
    app_data_ptr: *const u8,
    app_data_len: usize,
) -> i32

// Frame codecs (for BLE transport)
pub extern "C" fn lxmf_hdlc_encode(data_ptr: *const u8, data_len: usize, ...) -> i32
pub extern "C" fn lxmf_kiss_encode(data_ptr: *const u8, data_len: usize, ...) -> i32
```

### Memory/Pointer Safety

**Status Codes:**
```c
#define STATUS_OK        0
#define STATUS_ERR      -1
#define STATUS_NOT_INIT -2
#define STATUS_NOT_RUNNING -3
```

**Buffer Patterns:**
- Output buffers: caller allocates, passes capacity, function returns bytes written
- Input buffers: caller passes pointer + length
- Null pointers: checked explicitly (return error if null and required)
- String args: null-terminated C strings via `*const c_char`

**JSON Serialization:**
All query functions return JSON in caller-provided buffers:
- `lxmf_poll_events` → JSON array of events
- `lxmf_get_status` → JSON object with node status
- `lxmf_get_beacons` → JSON array of beacon objects

## iOS Integration (Swift)

### C Function Bindings

```swift
@_silgen_name("lxmf_init")
func lxmf_init(_ dbPath: UnsafePointer<CChar>?) -> Int32

@_silgen_name("lxmf_start")
func lxmf_start(
    _ identityPtr: UnsafePointer<UInt8>?,
    _ lxmfAddressPtr: UnsafePointer<UInt8>?,
    ...
) -> Int32

// ... etc for all exported functions
```

### Async Function Wrapper

```swift
AsyncFunction("start") { (
    identityHex: String,
    lxmfAddressHex: String,
    ...
) -> Bool in
    let identityBytes = Self.hexToBytes(identityHex)
    let addressBytes = Self.hexToBytes(lxmfAddressHex)
    
    return identityBytes.withUnsafeBufferPointer { idBuf in
        addressBytes.withUnsafeBufferPointer { addrBuf in
            lxmf_start(idBuf.baseAddress, addrBuf.baseAddress, ...) == 0
        }
    }
}
```

### Event Polling Loop

```swift
Timer.scheduledTimer(withTimeInterval: 0.08, repeats: true) { [weak self] _ in
    let len = self?.jsonBuf.withUnsafeMutableBufferPointer { buf in
        lxmf_poll_events(0, buf.baseAddress, buf.count)
    }
    
    if let len = len, len > 0 {
        let jsonData = Data(self!.jsonBuf[0..<Int(len)])
        if let events = try? JSONSerialization.jsonObject(with: jsonData) {
            for event in events {
                self?.sendEvent("on" + event.type, event)
            }
        }
    }
}
```

## Android Integration (Kotlin)

### JNI Declarations

```kotlin
private external fun nativeInit(dbPath: String?): Boolean
private external fun nativeStart(
    identityHex: String,
    lxmfAddressHex: String,
    ...
): Int
// ... etc
```

### JNI Stub Implementation (jni_bridge.rs)

```rust
#[no_mangle]
pub extern "C" fn Java_expo_modules_lxmf_LxmfModule_nativeStart(
    mut env: JNIEnv,
    _class: JClass,
    identity: JByteArray,
    lxmf_address: JByteArray,
    ...
) -> jint {
    let identity_bytes = env.convert_byte_array(&identity).ok()?;
    let mut id_arr: IdentityKey = [0u8; 32];
    id_arr.copy_from_slice(&identity_bytes[..32]);
    
    match LxmfNode::start(&id_arr, ...) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}
```

### Library Loading

```kotlin
companion object {
    init {
        System.loadLibrary("lxmf_rn")  // Loads liblxmf_rn.so
    }
}
```

## TypeScript Type Mapping

```typescript
// Rust i32 status → TS boolean/number
export function init(dbPath?: string | null): boolean
// Calls: lxmf_init → checks result == 0

// Rust u8* hex → TS string
export function send(destHex: string, bodyBase64: string): Promise<number>
// Converts: destHex → hexToBytes → u8[16] pointer

// Rust *mut u8 JSON buffer → TS object
export function getStatus(): string | null
// Returns: JSON string from buffer, parsed by useLxmf

// Arrays
export function broadcast(destsHex: string[], bodyBase64: string): Promise<number>
// Concatenates: string[] → u8[] (N * 16 bytes)
```

## Dependency Graph

```
lxmf_react_native_rust (workspace)
│
├── expo-module/
│   ├── package.json (TS + Native deps)
│   ├── ios/ (Swift + Expo Modules)
│   ├── android/ (Kotlin + JNI)
│   └── src/ (TypeScript)
│
└── rust-core/
    ├── Cargo.toml
    │   ├── rns-embedded-ffi (git: FreeTAKTeam/LXMF-rs)
    │   ├── rns-embedded-core
    │   ├── tokio
    │   ├── serde_json
    │   └── rusqlite
    │
    └── src/
        ├── lib.rs (crate-type: staticlib, cdylib)
        ├── ffi.rs (C exports)
        ├── node.rs (LxmfNode wrapper)
        ├── beacon.rs (Beacon state machine)
        ├── store.rs (SQLite)
        └── jni_bridge.rs (JNI stubs)
```

## Build Pipeline

```
1. cargo build --release (rust-core/)
   ├─ rns-embedded-ffi compiles
   └─ Outputs: liblxmf_rn.a (iOS), liblxmf_rn.so (Android)

2. xcodebuild (iOS app)
   ├─ Links liblxmf_rn.a via podspec
   ├─ Compiles LxmfModule.swift
   └─ App can call lxmf_* functions

3. gradle build (Android app)
   ├─ Copies liblxmf_rn.so to jniLibs/arm64-v8a
   ├─ Compiles LxmfModule.kt
   └─ System.loadLibrary("lxmf_rn") finds .so

4. npm install && npm run build (expo-module/)
   ├─ Compiles TypeScript → JavaScript
   └─ Type definitions available
```

## Testing the Integration

### Unit: Rust Compilation
```bash
cd rust-core && cargo test
```

### Integration: iOS Local Test
```swift
// In XCTest:
func testLxmfInit() {
    let result = lxmf_init(nil)
    XCTAssertEqual(result, 0)
}
```

### Integration: Android Local Test
```kotlin
// In AndroidTest:
@Test
fun testLxmfInit() {
    val result = nativeInit(null)
    Assert.assertTrue(result)
}
```

### E2E: React Native App
```tsx
const { start, send } = useLxmf();
// Run on physical device, verify BLE connections
```

---

**Key Takeaway:** The FFI layer is a thin transparent bridge. All real work happens in rns-embedded-ffi; the glue code just marshals pointers and enums across language boundaries.
