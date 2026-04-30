# @magicred-1/react-native-lxmf

LXMF mesh networking for React Native + Expo, powered by a Rust core.

Built by [anonme.sh](https://anonme.sh)

---

## What it is

Expo module wrapping a Rust implementation of [LXMF](https://github.com/markqvist/LXMF) over [Reticulum](https://reticulum.network). Runs on Android and iOS. Interoperable with Sideband, NomadNet, and other LXMF clients.

**Features:**
- BLE peer-to-peer mesh (no internet)
- TCP transport to `rnsd` (full Reticulum stack)
- RNode support (LoRa hardware via BLE)
- End-to-end encrypted messaging — LXMF wire-compatible
- Image and file attachments (LXMF standard fields)
- Opportunistic delivery queue — messages retry automatically when peer announces
- Large payload support — Link+Resource transfer for payloads > 464 B
- SQLite message persistence with full field schema
- Beacon discovery (anonmesh protocol)
- Identity persistence — export/import 128-char private key hex

## Install

```bash
npm install @magicred-1/react-native-lxmf
```

Requires a [custom dev client](https://docs.expo.dev/develop/development-builds/introduction/) — not compatible with Expo Go.

## Expo Plugin

Add to `app.json` to auto-configure BLE permissions:

```json
{
  "expo": {
    "plugins": ["@magicred-1/react-native-lxmf"]
  }
}
```

## Quick Start

```ts
import { useLxmf, LxmfNodeMode } from '@magicred-1/react-native-lxmf';
import * as SecureStore from 'expo-secure-store';
import { Buffer } from 'buffer';

export default function App() {
  const { start, stop, send, status, events, fetchMessages, isRunning } = useLxmf({
    dbPath: 'messages.db',
  });

  async function connect() {
    // Load or generate identity
    let identityHex = await SecureStore.getItemAsync('identity');
    let addressHex  = await SecureStore.getItemAsync('address');

    await start({
      identityHex:     identityHex ?? 'new',
      lxmfAddressHex: addressHex  ?? 'new',
      mode: LxmfNodeMode.Reticulum,
      tcpInterfaces: [{ host: 'my-rnsd-host', port: 4242 }],
      displayName: 'my-node',
    });
  }

  async function sendMessage(destHex: string, text: string) {
    const bodyBase64 = Buffer.from(text).toString('base64');
    await send(destHex, bodyBase64);
  }
}
```

## Transport Modes

| Mode | Value | Description |
|------|-------|-------------|
| `BleOnly` | 0 | BLE mesh only — no internet required |
| `TcpClient` | 1 | TCP client (non-standard framing) |
| `TcpServer` | 2 | TCP server (non-standard framing) |
| `Reticulum` | 3 | Full Reticulum stack via `rnsd` TCP |
| `ReticulumAndBle` | 4 | Reticulum TCP + BLE simultaneously |

BLE modes enforce a minimum 60-second announce interval to prevent TX queue saturation.

## API

### `useLxmf(options)`

```ts
const {
  // State
  status,          // LxmfNodeStatus | null
  beacons,         // Beacon[]
  events,          // LxmfEvent[]  (last 200, newest first)
  error,           // string | null
  isRunning,       // boolean
  isNativeAvailable,

  // Lifecycle
  start,           // (overrides?) => Promise<boolean>
  stop,            // () => Promise<void>

  // Messaging
  send,            // (destHex, bodyBase64, media?) => Promise<number>
  broadcast,       // (destsHex[], bodyBase64, media?) => Promise<number>
  fetchMessages,   // (limit?) => LxmfMessageEvent[]

  // Discovery
  getBeacons,      // () => Beacon[]

  // Identity
  getIdentityHex,  // () => string | null   ← persist to SecureStore

  // BLE
  startBLE, stopBLE, bleUnpairedRNodeCount,

  // Logging
  setLogLevel, getStatus,
} = useLxmf(options);
```

### Options

```ts
interface UseLxmfOptions {
  dbPath?:             string;           // SQLite file path for message persistence
  identityHex?:        string;           // 128-char private key hex, or 'new'
  lxmfAddressHex?:     string;           // 32-char address hex, or 'new'
  mode?:               LxmfNodeMode;     // default: BleOnly
  tcpInterfaces?:      TcpInterface[];   // required for Reticulum/TCP modes
  announceIntervalMs?: number;           // default: 60000 (BLE), 5000 (TCP)
  bleMtuHint?:         number;           // default: 255
  displayName?:        string;           // broadcast in announces
  isBeacon?:           boolean;          // advertise as anonmesh beacon
  autoStart?:          boolean;          // start automatically on mount
  logLevel?:           number;           // forward log events at or above this level
}
```

### Sending Messages

```ts
// Plain text
const bodyBase64 = Buffer.from('hello').toString('base64');
await send(destHex, bodyBase64);

// With image attachment (LXMF FIELD_IMAGE — rendered by Sideband etc.)
await send(destHex, bodyBase64, {
  image: {
    mimeType: 'image/jpeg',
    data: imageBase64,   // base64 string
  },
});

// With file attachments (LXMF FIELD_FILE_ATTACHMENTS)
await send(destHex, bodyBase64, {
  files: [
    { name: 'doc.pdf', data: fileBase64 },
  ],
});

// Broadcast to multiple destinations
await broadcast([dest1Hex, dest2Hex], bodyBase64, media);
```

### Message Persistence

`fetchMessages` returns stored messages matching the `LxmfMessageEvent` shape:

```ts
interface LxmfMessageEvent {
  id:        number;
  source:    string;    // sender address hex
  dest:      string;    // recipient address hex
  title:     string;    // base64
  body:      string;    // base64
  outbound:  boolean;
  timestamp: number;    // unix seconds
  acked:     boolean;
  image?:    { mimeType: string; data: string };   // data = base64
  files?:    { name: string; data: string }[];     // data = base64
}

const messages = fetchMessages(50);  // most recent 50
```

Messages are persisted automatically for both inbound (all transport modes) and outbound sends.

### Identity Persistence

```ts
const { getIdentityHex } = useLxmf();

// After start — save to encrypted storage
const hex = getIdentityHex();   // 128-char hex
await SecureStore.setItemAsync('identity', hex);

// On next mount — restore identity (same LXMF address)
const saved = await SecureStore.getItemAsync('identity');
await start({ identityHex: saved, lxmfAddressHex: savedAddress });
```

### Events

```ts
// Listen for incoming messages
const { events } = useLxmf({ ... });

events
  .filter(e => e.type === 'messageReceived')
  .forEach(e => {
    const body = Buffer.from(e.body, 'base64').toString('utf8');
    console.log('from', e.source, ':', body);
  });
```

Event types: `statusChanged`, `messageReceived`, `announceReceived`, `beaconDiscovered`, `packetReceived`, `txReceived`, `messageQueued`, `messageDelivered`, `messageFailed`, `log`, `error`.

## Architecture

```
React Native (TypeScript)
    ↓  useLxmf() hook
Expo Module (Swift / Kotlin)
    ↓  C FFI / JNI
Rust — rns-transport + LXMF encode/decode
    ↓
BLE mesh  |  TCP / rnsd  |  RNode (LoRa)
```

The Rust core handles:
- Identity generation and serialization
- LXMF msgpack encoding (wire-compatible with Sideband, NomadNet)
- Link+Resource transfer for payloads > 464 B (Reticulum MTU)
- Opportunistic outbound queue with SQLite persistence
- Announce-triggered delivery retry

## Interoperability

Wire-compatible with any LXMF client on the same Reticulum network:

- [Sideband](https://github.com/markqvist/Sideband)
- [NomadNet](https://github.com/markqvist/NomadNet)
- Any `rnsd`-connected node

## Repo

[github.com/magicred-1/react-native-lxmf](https://github.com/magicred-1/react-native-lxmf)

---

[anonme.sh](https://anonme.sh)
