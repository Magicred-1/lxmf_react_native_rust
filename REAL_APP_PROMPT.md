# Build the LXMF Messenger App

Build a production React Native (Expo) messenger on top of `@magicred-1/react-native-lxmf`.
The native layer — Rust core, BLE mesh, NUS/RNode, SQLite persistence, TCP/Reticulum — is complete.
Your job is the UI layer only.

---

## Install

```bash
npm install @magicred-1/react-native-lxmf expo-file-system
```

Requires a custom dev build (`npx expo run:android` / `npx expo run:ios`). Not Expo Go.

Add to `app.json` (auto-configures BLE permissions + iOS background modes):
```json
{ "expo": { "plugins": ["@magicred-1/react-native-lxmf"] } }
```

The plugin now also injects `UIBackgroundModes: [bluetooth-central, bluetooth-peripheral]`
into Info.plist automatically — background BLE works without manual plist edits.

---

## `useLxmf(options)` — full surface

```ts
import { useLxmf, LxmfModule, LxmfNodeMode } from '@magicred-1/react-native-lxmf';

const {
  // Reactive state
  isNativeAvailable,   // bool — false = not a dev build, show error screen
  isRunning,           // bool
  status,              // LxmfNodeStatus | null
  events,              // LxmfEvent[] — newest first, capped at 200
  beacons,             // Beacon[]
  error,               // string | null

  // Lifecycle
  start,               // (overrides?) => Promise<boolean>
  stop,                // () => Promise<void>  — also stops BLE automatically

  // Messaging
  send,                // (destHex, bodyBase64, media?) => Promise<number>  (-1 = error)
  broadcast,           // (destsHex[], bodyBase64, media?) => Promise<number>

  // Persistence
  fetchMessages,       // (limit?: number) => any[]  — from SQLite (see StoredMessage shape)

  // Identity
  getIdentityHex,      // () => string | null  — 128-char private key; persist to SecureStore

  // BLE
  bleUnpairedRNodeCount,  // () => number — RNodes seen in scan but not OS-bonded

  // Utilities
  getStatus,           // () => LxmfNodeStatus | null
  getBeacons,          // () => Beacon[]
  setLogLevel,         // (level: number) => void
} = useLxmf({
  dbPath?:             string,           // Raw filesystem path for SQLite (NOT a file:// URI)
  identityHex?:        string,           // 128-char private key hex, or 'new'
  lxmfAddressHex?:     string,           // 32-char address hex, or 'new'
  mode?:               LxmfNodeMode,     // default: BleOnly
  tcpInterfaces?:      { host: string; port: number }[], // required for Reticulum modes
  announceIntervalMs?: number,           // default: 60000 (BLE modes), 5000 (TCP-only)
  bleMtuHint?:         number,           // default: 255
  displayName?:        string,           // broadcast in announces
  isBeacon?:           boolean,          // advertise as anonmesh beacon
  autoStart?:          boolean,
  logLevel?:           number,
});
```

### dbPath — important

Pass a raw filesystem path (no `file://` prefix). Use `expo-file-system/legacy`:

```ts
import { documentDirectory } from 'expo-file-system/legacy';
const dbPath = (documentDirectory ?? '').replace('file://', '') + 'lxmf.db';
```

Without `dbPath`, SQLite runs in-memory — `fetchMessages` always returns `[]` and
messages are lost on restart.

---

## Sync module (call anywhere, no hook)

```ts
LxmfModule.blePeerCount()           // number — live BLE mesh peer count
LxmfModule.bleUnpairedRNodeCount()  // number — RNodes visible but not OS-bonded
LxmfModule.abiVersion()             // number
```

---

## Transport modes

```ts
enum LxmfNodeMode {
  BleOnly         = 0,  // BLE mesh — no internet
  TcpClient       = 1,  // TCP (non-standard framing)
  TcpServer       = 2,  // TCP server
  Reticulum       = 3,  // full rnsd TCP
  ReticulumAndBle = 4,  // rnsd TCP + BLE simultaneously
}
```

**BLE auto-lifecycle**: `start()` starts BLE automatically. `stop()` stops it.
Do NOT call `startBLE()`/`stopBLE()` manually.

**BLE cross-platform**: iOS ↔ Android BLE mesh works — both platforms share the same
GATT service UUIDs and Rust framing logic. Both roles (central + peripheral) run
simultaneously on each device.

**BLE announce interval**: clamped to 60s minimum by Rust.

---

## `start()` overrides

```ts
// BLE only
await start({ mode: LxmfNodeMode.BleOnly, displayName: 'alice' });

// TCP + BLE
await start({
  mode: LxmfNodeMode.ReticulumAndBle,
  tcpInterfaces: [{ host: '192.168.1.10', port: 4242 }],
  displayName: 'alice',
});

// TCP only (rnsd)
await start({
  mode: LxmfNodeMode.Reticulum,
  tcpInterfaces: [{ host: '192.168.1.10', port: 4242 }],
  displayName: 'alice',
});
```

Modes 1–4 require at least one entry in `tcpInterfaces`.

---

## Events

```ts
{ type: 'announceReceived',  destHash, appData, hops, timestamp }
{ type: 'messageReceived',   source, title, body, timestamp, image?, files? }
  // title and body are base64-encoded UTF-8
{ type: 'beaconDiscovered',  destHash, ... }
{ type: 'statusChanged',     running, ... }
{ type: 'log',               message, level }
{ type: 'error',             message, code }
{ type: 'messageQueued',     seq, dest_hex }   // message queued for retry (peer offline)
```

---

## Message persistence (`fetchMessages`)

Returns messages from SQLite — survives restarts. Shape:

```ts
interface StoredMessage {
  id:        number;
  source:    string;    // 32-char hex — sender address
  dest:      string;    // 32-char hex — recipient address
  title:     string;    // base64 (may be empty)
  body:      string;    // base64
  outbound:  boolean;   // true = sent by us, false = received
  timestamp: number;    // unix seconds
  acked:     boolean;   // delivery acknowledged
  image?:    { mimeType: string; data: string };   // data = base64
  files?:    { name: string; data: string }[];
}
```

Filter by thread: `msgs.filter(m => m.source === addr || m.dest === addr)`

**Both inbound and outbound messages persist automatically.**

---

## Sending

```ts
// Encode text to base64 (handles UTF-8 correctly)
function b64(s: string): string {
  return globalThis.btoa(
    Array.from(new TextEncoder().encode(s), b => String.fromCodePoint(b)).join('')
  );
}

// Plain text
await send(destHex, b64('hello'));

// With image (LXMF FIELD_IMAGE — rendered by Sideband etc.)
await send(destHex, b64('see pic'), {
  image: { mimeType: 'image/jpeg', data: jpegBase64 },
});

// With files
await send(destHex, b64('see attachment'), {
  files: [{ name: 'doc.pdf', data: pdfBase64 }],
});

// Broadcast to all known peers
await broadcast([dest1, dest2, dest3], b64('hey everyone'));
```

`send()` returns a sequence number ≥ 0 on success (message may be queued if peer is
offline), or -1 on error. Queued messages retry automatically on next peer announce.

---

## Identity persistence pattern

```ts
import { documentDirectory } from 'expo-file-system/legacy';
import * as SecureStore from 'expo-secure-store';

const DB_PATH = (documentDirectory ?? '').replace('file://', '') + 'lxmf.db';

// First launch: pass 'new' → Rust generates identity
const lxmf = useLxmf({ identityHex: 'new', lxmfAddressHex: 'new', dbPath: DB_PATH });
await lxmf.start({ displayName: 'alice' });

// Save after start
const idHex   = lxmf.getIdentityHex();       // 128-char private key hex
const addrHex = lxmf.status?.addressHex;     // 32-char public address

await SecureStore.setItemAsync('lxmf.identity', JSON.stringify({
  version: 1, identity_hex: idHex, address_hex: addrHex, created_at: new Date().toISOString()
}));

// Subsequent launches: restore
const stored = JSON.parse(await SecureStore.getItemAsync('lxmf.identity') ?? 'null');
const lxmf = useLxmf({
  identityHex:    stored?.identity_hex ?? 'new',
  lxmfAddressHex: stored?.address_hex  ?? 'new',
  dbPath: DB_PATH,
});
```

**Never display, log, or share `identityHex`** — it is the private key.
Only show `status.addressHex` (public address) to the user.

---

## `LxmfNodeStatus` shape

```ts
interface LxmfNodeStatus {
  running:              boolean;
  mode:                 number;
  identityHex:          string;   // private — never show
  addressHex:           string;   // public address — safe to display
  lifecycle:            number;
  epoch:                number;
  pendingOutbound:      number;
  outboundSent:         number;
  inboundAccepted:      number;
  announcesReceived:    number;
  lxmfMessagesReceived: number;
  blePeerCount:         number;
}
```

---

## Recommended architecture

Call `useLxmf` **once** at the root in a React Context provider. Pass everything
downstream via context. This avoids multiple native polling loops.

```tsx
// context/LxmfContext.tsx
import { documentDirectory } from 'expo-file-system/legacy';
const DB_PATH = (documentDirectory ?? '').replace('file://', '') + 'lxmf.db';

export function LxmfProvider({ children }) {
  const lxmf = useLxmf({ dbPath: DB_PATH, identityHex: storedIdHex, ... });
  return <LxmfContext.Provider value={lxmf}>{children}</LxmfContext.Provider>;
}

// app/_layout.tsx
export default function RootLayout() {
  return <LxmfProvider><Stack /></LxmfProvider>;
}
```

---

## Constraints

- **464 B LXMF packet MTU** over BLE. Larger payloads use Link+Resource transfer
  automatically, but warn users if body > 200 chars in BLE-only mode.
- **RNode pairing**: user must pair RNode (Heltec V3 etc.) in iOS/Android Bluetooth
  settings first. `bleUnpairedRNodeCount()` detects nearby unpaired RNodes. Show prompt.
- **Android BLE permissions** (API 31+): request `BLUETOOTH_SCAN`, `BLUETOOTH_ADVERTISE`,
  `BLUETOOTH_CONNECT` before calling `start()`. API < 31: `ACCESS_FINE_LOCATION`.
- **Must be a dev build.** Show a full-screen error if `isNativeAvailable` is false.
- **Outbound queue**: messages sent while peer is offline queue in SQLite and retry
  automatically when the peer next announces. No retry logic needed in the app.

---

## App to build

### Stack

- Expo Router (file-based)
- `expo-secure-store` — identity persistence
- `expo-file-system/legacy` — documentDirectory for dbPath
- `expo-image-picker` — image attachments
- `@shopify/flash-list` — message lists (perf > FlatList)
- No Redux/Zustand — `useLxmf` via Context is source of truth

### File structure

```
context/
  LxmfContext.tsx           ← useLxmf singleton + identity + contacts
app/
  _layout.tsx               ← wrap with LxmfProvider
  (tabs)/
    _layout.tsx             ← 3 tabs: Messages, Network, Settings
    conversations.tsx       ← contact list + recent messages
    network.tsx             ← transport controls + node status
    settings.tsx            ← identity, display name, log level
  conversation/
    [address].tsx           ← message thread
```

---

## Tab: Conversations

Primary screen. Contact list sorted by most recent activity.

**Contact** = any LXMF address seen from:
1. `announceReceived` events — `e.destHash` is the peer address
2. `messageReceived` events — `e.source` is the sender
3. Manual entry by user (32-char hex)

Persist contacts `{ address, name, lastSeen, lastMessage, unread }` in SecureStore as JSON.
Update on each event. `name` comes from announce `appData`.

**Contact row** shows:
- Name (from `appData`) or `addr[:6]…addr[-6:]` if no name
- Last message preview (decoded from base64, 60 chars)
- Relative timestamp ("2m ago")
- Unread badge — count since thread last opened

**FAB** bottom-right: text input for 32-char hex address.

Empty state: "No contacts yet — start node in Network tab."

---

## Screen: Message Thread (`conversation/[address].tsx`)

Standard messenger UI. Oldest message at top, newest at bottom.

**Data: merge and dedup by id/timestamp:**
1. `fetchMessages(100)` on mount + on new `messageReceived` event — SQLite history
2. Live events filtered to `e.type === 'messageReceived' && e.source === address`
3. Filter SQLite by `m.source === address || m.dest === address`
4. Sort ascending by `timestamp`

**Bubble layout:**
- `outbound: true` → right-aligned, accent color `#1a7fc1`
- `outbound: false` → left-aligned, surface color `#1a2a38`
- Title: italic above body (omit if empty, decoded from base64)
- Body: decoded from base64
- `image` present → `[Image: mime/type]` tappable badge → full-screen via `expo-image`
- `files` present → `[N file(s)]` badge
- `acked: true` on outbound → small ✓ after timestamp
- Timestamp bottom corner of bubble

**Compose bar** (sticky bottom, `KeyboardAvoidingView`):
- Multiline `TextInput`
- Warn at 180+ chars in BLE-only mode
- Image attach icon → `expo-image-picker` → base64 → `media.image`
- Send button → `send(address, b64(text), media)` → on success clear input, refresh history

**On send failure** (`r === -1`): show "Send failed — message queued for retry."
(Message is already in SQLite outbound queue, will deliver on next announce.)

**Header**: peer name + truncated address. Back button.
**On mount**: call `markRead(address)` to clear unread badge.

---

## Tab: Network

Two cards: **Transport** and **Node Status**.

#### Transport card

Segment control: **BLE** | **TCP** | **TCP+BLE**

**BLE tab:**
```
Display name:  [TextInput default "lxmf-mobile"]
[Start BLE]  [Stop]
```
Before `start()` on Android: request BLE permissions.
After start: poll `LxmfModule.blePeerCount()` every 2s.
If `LxmfModule.bleUnpairedRNodeCount() > 0`: yellow banner:
> "Found N RNode(s) nearby. Pair in Settings > Bluetooth first."

**TCP tab:**
```
Host:  [TextInput "192.168.1.135"]
Port:  [TextInput "4242"]
Display name: [TextInput]
[Start Reticulum]  [Stop]
```

**TCP+BLE tab:** same as TCP. Calls `start({ mode: ReticulumAndBle, ... })`.

#### Node Status card

Refresh every 5s via `getStatus()`:
```
State:            ● Running
Mode:             ReticulumAndBle
Address:          abc123…xyz789  [copy]
BLE peers:        3
Pending outbound: 0
Messages sent:    4
Messages received: 7
Announces:        12
```

---

## Tab: Settings

```
Display name       [TextInput]    persisted, used as default in start()

My address         abc123…xyz789  [copy icon]   ← addressHex only, never identityHex
Full address       <selectable full 32-char hex>
Created            2026-04-01

Log level          [− 3 – Info +]

─────────────────────────────────────────────
[Reset Identity]  → confirm modal:
  "This permanently deletes your identity and address.
   You will lose access to all pending messages."
  → clears SecureStore → next start() generates new identity
  Disabled while node is running.
```

---

## UX rules

- Dark theme only. `bg: #0c1218`, `surface: #131d26`, `border: #1e3040`, `accent: #1a7fc1`.
- `isNativeAvailable === false` → full-screen: "Run in a dev build, not Expo Go."
- `error` set → dismissible red banner top of screen.
- Address display: always `first6…last6`. Full selectable address only in thread header + Settings.
- Use `Pressable` not `TouchableOpacity`.
- No skeleton loaders — short empty state copy is fine.
- `send()` returning ≥ 0 does NOT mean delivered — message may be queued. Don't show "sent ✓"
  until `acked: true` appears in `fetchMessages`.

## What NOT to build in v1

- Push notifications
- Group DM threads (broadcast = announcements, not group chat)
- File open/download
- User accounts or servers
- Web/desktop targets
