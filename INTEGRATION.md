# LXMF React Native Rust Integration

Complete mobile bridge for LXMF/Reticulum mesh networking via Expo Modules, connecting React Native to Rust via C FFI and JNI.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    TypeScript / React Native                     │
│  useLxmf() hook + LxmfModule native reference                   │
└────────────────┬────────────────────────────────────────────────┘
                 │ (JSON events via NativeEventEmitter)
┌────────────────┴────────────────────────────────────────────────┐
│           Expo Native Modules Layer                              │
│  ┌──────────────────────┐  ┌──────────────────────┐             │
│  │  iOS (Swift)         │  │  Android (Kotlin)    │             │
│  │  LxmfModule.swift    │  │  LxmfModule.kt       │             │
│  │  BLEManager.swift    │  │  + JNI stubs         │             │
│  └──────────┬───────────┘  └──────────┬───────────┘             │
│             │                         │                          │
│             └─────────┬───────────────┘                          │
│                       │                                          │
└───────────────────────┼──────────────────────────────────────────┘
                        │ (C FFI calls)
┌───────────────────────┴──────────────────────────────────────────┐
│         Rust Core (rust-core/src/)                               │
│                                                                   │
│  ffi.rs: C FFI exports                                          │
│  node.rs: LxmfNode wrapper around rns-embedded-ffi-v1           │
│  beacon.rs: Beacon announce/discovery state machine             │
│  store.rs: SQLite message persistence                           │
│  jni_bridge.rs: JNI↔Rust glue                                   │
└───────────────────────┬──────────────────────────────────────────┘
                        │ (rns-embedded-ffi crate)
┌───────────────────────┴──────────────────────────────────────────┐
│    rns-embedded-ffi v1 (FreeTAKTeam/LXMF-rs)                     │
│                                                                   │
│  - BLE mesh (Bluetooth Core)                                     │
│  - LoRa transport (if available)                                 │
│  - TCP/UDP fallback                                              │
│  - X25519 + AES-256-GCM encryption                              │
│  - Announce/link lifecycle management                            │
└───────────────────────────────────────────────────────────────────┘
```

## Quick Start

### 1. Build Rust Core

```bash
cd rust-core
cargo build --release
```

Produces:
- `target/release/liblxmf_rn.a` (iOS staticlib)
- `target/release/liblxmf_rn.so` (Android shared library)

### 2. Install Expo Module

```bash
cd expo-module
npm install
npm run build
```

### 3. Use in React Native App

```tsx
import { useLxmf } from '@lxmf/react-native';
import { useEffect } from 'react';

export default function App() {
  const { start, stop, send, status, error } = useLxmf({
    logLevel: 2, // Info
  });

  useEffect(() => {
    // Start the node with identity and LXMF address
    const identity = '0'.repeat(64); // 32 bytes in hex
    const address = '0'.repeat(32);  // 16 bytes in hex
    start(identity, address, 0); // mode 0 = BLE only
  }, [start]);

  return (
    <View>
      {status && <Text>Running: {status.running}</Text>}
      {error && <Text style={{ color: 'red' }}>{error}</Text>}
    </View>
  );
}
```

## Building for iOS

The Swift module (`LxmfModule.swift`) uses Expo Modules API to:
1. Import the C FFI functions as `@_silgen_name` declarations
2. Call the Rust library via C ABI
3. Poll events from Rust at 80ms intervals
4. Emit Swift events to React Native

The `BLEManager.swift` handles:
- Dual-role BLE (central + peripheral)
- Service discovery and connection
- RX/TX characteristic handling
- Frame encoding (HDLC/KISS for BLE transport)

### Prerequisites

- Xcode 14+
- Deployment target: iOS 13+
- CoreBluetooth framework

### Linking

The Rust library is linked via CocoaPods (`LxmfReactNative.podspec`):
- Vendored libraries: `liblxmf_rn.a`
- Frameworks: CoreBluetooth, Foundation

## Building for Android

The Kotlin module (`LxmfModule.kt`) uses:
1. JNI stubs that call Rust FFI via `System.loadLibrary("lxmf_rn")`
2. Expo modules for event emission to JS
3. Same event polling as iOS

### Prerequisites

- Android NDK (r25+)
- Android API 24+
- Gradle 8.0+

### Linking

Build config (`build.gradle.kts`):
- Copies `liblxmf_rn.so` to `src/main/jniLibs/arm64-v8a`
- Links C++ runtime
- Exports JNI functions via `expo.modules.lxmf.LxmfModule`

## TypeScript API

### `useLxmf(options)`

React hook for LXMF node lifecycle and messaging.

**Options:**
- `autoStart?: boolean` — automatically start on mount
- `identityHex?: string` — 64-char hex (32 bytes)
- `lxmfAddressHex?: string` — 32-char hex (16 bytes)
- `dbPath?: string` — SQLite file path (optional)
- `logLevel?: number` — 0=error, 1=warn, 2=info, 3=debug

**Returns:**
- `status: LxmfNodeStatus | null` — current node state
- `beacons: Beacon[]` — discovered peer beacons
- `events: LxmfEvent[]` — recent events (polled)
- `error: string | null` — last error message
- `isRunning: boolean` — node running flag
- `start(identity, address, mode)` — start the node
- `stop()` — stop the node
- `send(destHex, bodyBase64)` — send to single peer
- `broadcast(destsHex[], bodyBase64)` — send to multiple peers
- `getStatus()` — fetch current status JSON
- `getBeacons()` — fetch beacon pool
- `fetchMessages(limit)` — fetch persisted messages
- `setLogLevel(level)` — adjust log verbosity
- `startBLE() / stopBLE()` — BLE radio control

## File Structure

```
.
├── rust-core/                 # Rust FFI bridge
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── ffi.rs             # C FFI exports
│       ├── node.rs            # LxmfNode wrapper
│       ├── beacon.rs          # Beacon state machine
│       ├── store.rs           # SQLite persistence
│       ├── jni_bridge.rs      # JNI stubs
│       └── framing.rs         # HDLC/KISS codecs
│
├── expo-module/               # Expo module + TypeScript
│   ├── package.json
│   ├── tsconfig.json
│   ├── expo-module.config.js
│   ├── LxmfReactNative.podspec
│   ├── src/
│   │   ├── index.ts           # Main export
│   │   ├── LxmfModule.ts      # Native module wrapper
│   │   └── useLxmf.ts         # React hook
│   ├── ios/
│   │   ├── LxmfModule.swift   # Expo module + polling
│   │   └── BLEManager.swift   # Dual-role BLE
│   └── android/
│       ├── build.gradle.kts
│       └── src/main/kotlin/
│           └── expo/modules/lxmf/
│               └── LxmfModule.kt
│
└── README.md (this file)
```

## Event Flow Example

```
User calls: send("deadbeef...", "aGVsbG8=")
                    ↓
         [useLxmf.send()]
                    ↓
        [LxmfModule.send() JS→Native]
                    ↓
         [LxmfModule.swift/kt]
                    ↓
         [ffi.rs: lxmf_send()]
                    ↓
      [Rust: node.rs LxmfNode::send()]
                    ↓
    [rns-embedded-ffi: send via BLE/mesh]
                    ↓
        [Event polled at 80ms interval]
                    ↓
      [Swift: drainEvents() → JSON]
                    ↓
    [JS: onPacketReceived event emitted]
                    ↓
      [useLxmf state updated]
                    ↓
       [React component re-renders]
```

## Integration Checklist

- [x] Rust core compiles to static/shared libs
- [x] iOS Swift module with FFI bindings
- [x] Android Kotlin module with JNI stubs
- [x] TypeScript native module wrapper
- [x] React `useLxmf` hook
- [ ] Example React Native app
- [ ] Unit tests (Jest + Swift XCTest)
- [ ] Integration tests (mock BLE)
- [ ] CI/CD pipeline (GitHub Actions)

## Development

### Testing the Rust Build

```bash
cd rust-core
cargo check
cargo test
```

### Testing iOS Locally

```bash
# In Xcode:
1. Open rust-core/Cargo.toml in Xcode's build settings
2. Add liblxmf_rn.a to Link Binary With Libraries
3. Run LxmfModuleTests in XCTest
```

### Testing Android Locally

```bash
cd expo-module/android
./gradlew build
./gradlew connectedAndroidTest
```

### TypeScript Type Checking

```bash
cd expo-module
npm run type-check
```

## License

MIT

## References

- [LXMF-rs](https://github.com/FreeTAKTeam/LXMF-rs)
- [Reticulum Specification](https://reticulum.network/docs/)
- [Expo Modules Core](https://docs.expo.dev/modules/overview/)
- [React Native BLE Docs](https://reactnative.dev/docs/native-modules-intro)
