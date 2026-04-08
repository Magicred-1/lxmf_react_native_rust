import React, { useCallback, useEffect, useMemo, useState } from 'react';
import {
  RefreshControl,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  TouchableOpacity,
  View,
} from 'react-native';
import { LxmfEvent, LxmfNodeMode, LxmfNodeStatus, useLxmf } from '@lxmf/react-native';

function truncHex(hex: string, len = 8): string {
  if (!hex) return '—';
  return hex.length > len * 2 ? `${hex.slice(0, len)}...${hex.slice(-len)}` : hex;
}

const BASE64_ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';

function bytesToBase64(bytes: Uint8Array): string {
  let output = '';
  for (let i = 0; i < bytes.length; i += 3) {
    const b0 = bytes[i] ?? 0;
    const b1 = bytes[i + 1] ?? 0;
    const b2 = bytes[i + 2] ?? 0;
    const triplet = (b0 << 16) | (b1 << 8) | b2;

    output += BASE64_ALPHABET[(triplet >> 18) & 0x3f];
    output += BASE64_ALPHABET[(triplet >> 12) & 0x3f];
    output += i + 1 < bytes.length ? BASE64_ALPHABET[(triplet >> 6) & 0x3f] : '=';
    output += i + 2 < bytes.length ? BASE64_ALPHABET[triplet & 0x3f] : '=';
  }
  return output;
}

function utf8ToBase64(input: string): string {
  if (typeof globalThis.btoa === 'function') {
    return globalThis.btoa(input);
  }
  if (typeof TextEncoder !== 'undefined') {
    return bytesToBase64(new TextEncoder().encode(input));
  }
  return input;
}

function hexToBytes(hex: string): Uint8Array {
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < hex.length; i += 2) {
    bytes[i / 2] = Number.parseInt(hex.slice(i, i + 2), 16);
  }
  return bytes;
}

function bytesToUtf8(bytes: Uint8Array): string {
  if (typeof TextDecoder !== 'undefined') {
    return new TextDecoder('utf-8', { fatal: false }).decode(bytes);
  }
  return String.fromCharCode(...Array.from(bytes));
}


/**
 * Minimal msgpack decoder for the LXMF message format:
 *   [timestamp: float, title: str|bin, content: str|bin, fields: map|nil]
 */
function decodeLxmfMsgpack(bytes: Uint8Array): { timestamp: number; title: string; content: string } | null {
  let pos = 0;

  const readByte = (): number => {
    if (pos >= bytes.length) throw new Error('eof');
    return bytes[pos++]!;
  };
  const readSlice = (n: number): Uint8Array => {
    const s = bytes.slice(pos, pos + n);
    pos += n;
    return s;
  };
  const readU16 = (): number => { const b = readSlice(2); return (b[0]! << 8) | b[1]!; };
  const readU32 = (): number => { const b = readSlice(4); return ((b[0]! << 24) | (b[1]! << 16) | (b[2]! << 8) | b[3]!) >>> 0; };
  const readF64 = (): number => {
    const b = readSlice(8);
    const v = new DataView(b.buffer, b.byteOffset, 8);
    return v.getFloat64(0, false);
  };
  const readF32 = (): number => {
    const b = readSlice(4);
    const v = new DataView(b.buffer, b.byteOffset, 4);
    return v.getFloat32(0, false);
  };

  function readValue(): unknown {
    const t = readByte();
    if (t <= 0x7f) return t;                             // positive fixint
    if (t >= 0xe0) return t - 256;                       // negative fixint
    if (t >= 0xa0 && t <= 0xbf) return bytesToUtf8(readSlice(t & 0x1f)); // fixstr
    if (t >= 0x90 && t <= 0x9f) { const n = t & 0x0f; return Array.from({ length: n }, () => readValue()); } // fixarray
    if (t >= 0x80 && t <= 0x8f) { // fixmap
      const n = t & 0x0f;
      const m: Record<string, unknown> = {};
      for (let i = 0; i < n; i++) { const k = readValue(); m[String(k)] = readValue(); }
      return m;
    }
    switch (t) {
      case 0xc0: return null;
      case 0xc2: return false;
      case 0xc3: return true;
      case 0xc4: return bytesToUtf8(readSlice(readByte()));      // bin8 → string
      case 0xc5: return bytesToUtf8(readSlice(readU16()));       // bin16
      case 0xc6: return bytesToUtf8(readSlice(readU32()));       // bin32
      case 0xca: return readF32();
      case 0xcb: return readF64();
      case 0xcc: return readByte();                              // uint8
      case 0xcd: return readU16();                              // uint16
      case 0xce: return readU32();                              // uint32
      case 0xd0: { const b = readByte(); return b > 127 ? b - 256 : b; } // int8
      case 0xd1: { const v = readU16(); return v > 0x7fff ? v - 0x10000 : v; }
      case 0xd2: { const v = readU32(); return v > 0x7fffffff ? v - 0x100000000 : v; }
      case 0xd9: return bytesToUtf8(readSlice(readByte()));      // str8
      case 0xda: return bytesToUtf8(readSlice(readU16()));       // str16
      case 0xdb: return bytesToUtf8(readSlice(readU32()));       // str32
      case 0xdc: { const n = readU16(); return Array.from({ length: n }, () => readValue()); } // array16
      case 0xdd: { const n = readU32(); return Array.from({ length: n }, () => readValue()); } // array32
      default: return null;
    }
  }

  try {
    const val = readValue();
    if (!Array.isArray(val) || val.length < 3) return null;
    return {
      timestamp: typeof val[0] === 'number' ? val[0] : 0,
      title: typeof val[1] === 'string' ? val[1] : '',
      content: typeof val[2] === 'string' ? val[2] : '',
    };
  } catch {
    return null;
  }
}

/**
 * LXMF wire format: [16B dest hash][16B source hash][64B Ed25519 sig][msgpack payload]
 * msgpack payload: [timestamp: f64, title: bytes, content: bytes, fields: map, stamp?: bytes]
 */
const LXMF_HEADER_BYTES = 96; // 16 + 16 + 64

/** Extract the sender's LXMF address (bytes 16-31) from a raw LXMF message. */
function lxmfSourceHex(bytes: Uint8Array): string | null {
  if (bytes.length < LXMF_HEADER_BYTES) return null;
  return Array.from(bytes.slice(16, 32))
    .map((b) => b.toString(16).padStart(2, '0'))
    .join('');
}

/** Decode a hex-encoded LXMF message payload into readable text. */
function decodeLxmfContent(hexContent: string): string {
  if (!hexContent || hexContent.length % 2 !== 0 || !/^[0-9a-fA-F]+$/.test(hexContent)) {
    return '—';
  }
  try {
    const bytes = hexToBytes(hexContent);

    // Try LXMF wire format: skip 96-byte header, parse msgpack payload
    if (bytes.length > LXMF_HEADER_BYTES) {
      const payload = bytes.slice(LXMF_HEADER_BYTES);
      const lxmf = decodeLxmfMsgpack(payload);
      if (lxmf) {
        const parts: string[] = [];
        if (lxmf.title.trim()) parts.push(`[${lxmf.title.trim()}]`);
        if (lxmf.content.trim()) parts.push(lxmf.content.trim());
        return parts.length > 0 ? parts.join(' ') : '(empty)';
      }
    }

    // Fall back: messages sent by our own app are raw UTF-8 (no LXMF header)
    return bytesToUtf8(bytes).trim() || '(empty)';
  } catch {
    return '(decode error)';
  }
}

const MODE_LABELS: Record<number, string> = {
  [LxmfNodeMode.BleOnly]: 'BLE Only',
  [LxmfNodeMode.TcpClient]: 'TCP Client (FFI)',
  [LxmfNodeMode.TcpServer]: 'TCP Server (FFI)',
  [LxmfNodeMode.Reticulum]: 'Reticulum TCP',
};

interface HoldAnnounceEntry {
  key: string;
  destination: string;
  iface: string;
  holdSeconds: string;
  at: string;
}

interface MissingIdentityEntry {
  key: string;
  destination: string;
  at: string;
}

function isTcpTransportMode(mode: LxmfNodeMode): boolean {
  return mode === LxmfNodeMode.Reticulum || mode === LxmfNodeMode.TcpClient || mode === LxmfNodeMode.TcpServer;
}

function getTcpHostLabel(mode: LxmfNodeMode): string {
  if (mode === LxmfNodeMode.Reticulum) return 'rnsd host';
  if (mode === LxmfNodeMode.TcpClient) return 'Connect to host';
  return 'Bind address';
}

function getModeInfo(mode: LxmfNodeMode, tcpHost: string, tcpPort: string): string {
  if (mode === LxmfNodeMode.BleOnly) {
    return 'BLE Only: Bluetooth Low Energy mesh. No internet required.';
  }
  if (mode === LxmfNodeMode.Reticulum) {
    return `Reticulum TCP: connects to rnsd at ${tcpHost}:${tcpPort}. Full protocol — real identity, announces, routing, visible to all Reticulum nodes.`;
  }
  if (mode === LxmfNodeMode.TcpClient) {
    return `FFI TCP Client: internal framing to ${tcpHost}:${tcpPort}. Only for rns-embedded-ffi peers.`;
  }
  return `FFI TCP Server: listens on ${tcpHost}:${tcpPort}. Only for rns-embedded-ffi peers.`;
}

function getNextMode(mode: LxmfNodeMode): LxmfNodeMode {
  if (mode === LxmfNodeMode.Reticulum) return LxmfNodeMode.BleOnly;
  if (mode === LxmfNodeMode.BleOnly) return LxmfNodeMode.TcpClient;
  if (mode === LxmfNodeMode.TcpClient) return LxmfNodeMode.TcpServer;
  return LxmfNodeMode.Reticulum;
}


function getMessageKey(m: LxmfEvent): string {
  return `${String(m.source ?? 'unknown')}:${String(m.timestamp ?? '')}:${String(m.content ?? '')}`;
}

function getLineMap(lines: string[]): { key: string; line: string }[] {
  const counts = new Map<string, number>();
  return lines.map((line) => {
    const count = (counts.get(line) ?? 0) + 1;
    counts.set(line, count);
    return { key: `${line}::${count}`, line };
  });
}

function parseHoldAnnounceLog(line: string, at: string): HoldAnnounceEntry | null {
  const holdRegex = /holding announce for \/([0-9a-fA-F]+)\/ on iface \/([0-9a-fA-F]+)\/ for at least ([0-9.]+)s/;
  const match = holdRegex.exec(line);
  if (!match) {
    return null;
  }
  const destination = match[1] ?? '';
  const iface = match[2] ?? '';
  const holdSeconds = match[3] ?? '';

  return {
    key: `${destination}:${iface}:${holdSeconds}:${at}`,
    destination,
    iface,
    holdSeconds,
    at,
  };
}

function parseMissingIdentityLog(line: string, at: string): MissingIdentityEntry | null {
  const missingRegex = /missing destination identity for \/([0-9a-fA-F]+)\//;
  const match = missingRegex.exec(line);
  if (!match) {
    return null;
  }
  const destination = match[1] ?? '';
  return {
    key: `${destination}:${at}`,
    destination,
    at,
  };
}

function getResolvingDestinations(
  holdAnnounces: HoldAnnounceEntry[],
  missingIdentities: MissingIdentityEntry[],
): Set<string> {
  const resolving = new Set<string>();
  holdAnnounces.forEach((entry) => resolving.add(entry.destination.toLowerCase()));
  missingIdentities.forEach((entry) => resolving.add(entry.destination.toLowerCase()));
  return resolving;
}

function processIncomingEvents(
  events: LxmfEvent[],
  setEventLog: React.Dispatch<React.SetStateAction<string[]>>,
  setRustLogs: React.Dispatch<React.SetStateAction<string[]>>,
  setHoldAnnounces: React.Dispatch<React.SetStateAction<HoldAnnounceEntry[]>>,
  setMissingIdentities: React.Dispatch<React.SetStateAction<MissingIdentityEntry[]>>,
  setAnnounces: React.Dispatch<React.SetStateAction<LxmfEvent[]>>,
  setMessages: React.Dispatch<React.SetStateAction<LxmfEvent[]>>,
) {
  const now = new Date().toLocaleTimeString();
  const newEntries = events.map((event: LxmfEvent) => {
    if (event.type === 'log') {
      return `[${now}] log[L${String(event.level ?? '?')}] ${String(event.message ?? '')}`;
    }
    return `[${now}] ${event.type}: ${JSON.stringify(event).slice(0, 120)}`;
  });
  setEventLog((prev) => [...newEntries, ...prev].slice(0, 100));

  const newRustLogs = events
    .filter((event: LxmfEvent) => event.type === 'log' && typeof event.message === 'string')
    .map((event: LxmfEvent) => {
      const level = typeof event.level === 'number' ? event.level : '?';
      return `[${now}] [L${level}] ${String(event.message)}`;
    });

  if (newRustLogs.length > 0) {
    setRustLogs((prev) => [...newRustLogs, ...prev].slice(0, 240));

    const parsed = newRustLogs
      .map((line: string) => parseHoldAnnounceLog(line, now))
      .filter((entry: HoldAnnounceEntry | null): entry is HoldAnnounceEntry => entry !== null);

    setHoldAnnounces((prev) => (parsed.length > 0 ? [...parsed, ...prev].slice(0, 100) : prev));

    const missing = newRustLogs
      .map((line: string) => parseMissingIdentityLog(line, now))
      .filter((entry: MissingIdentityEntry | null): entry is MissingIdentityEntry => entry !== null);

    setMissingIdentities((prev) => (missing.length > 0 ? [...missing, ...prev].slice(0, 80) : prev));
  }

  const newAnnounces = events.filter((event: LxmfEvent) => event.type === 'announceReceived');
  setAnnounces((prev) => (newAnnounces.length > 0 ? [...newAnnounces, ...prev].slice(0, 50) : prev));

  const newMessages = events.filter((event: LxmfEvent) => event.type === 'messageReceived');
  setMessages((prev) => (newMessages.length > 0 ? [...newMessages, ...prev].slice(0, 100) : prev));
}

async function sendMessage(
  send: (destinationHex: string, bodyB64: string) => Promise<number>,
  sendDest: string,
  sendBody: string,
  setSendResult: React.Dispatch<React.SetStateAction<string | null>>,
) {
  if (!sendDest) {
    setSendResult('Enter a destination hex');
    return;
  }
  const bodyB64 = utf8ToBase64(sendBody);
  const receipt = await send(sendDest, bodyB64);
  if (receipt >= 0) {
    setSendResult(`Sent! receipt #${receipt}`);
  } else {
    setSendResult('Send failed — check that the destination has announced and the node is running.');
  }
}

function ErrorBanner({ error }: { readonly error: string | null }) {
  if (!error) {
    return null;
  }
  return (
    <View style={styles.errorCard}>
      <Text style={styles.errorText}>{error}</Text>
    </View>
  );
}

function NodeStatusDetails({
  nodeStatus,
  isRunning,
}: {
  readonly nodeStatus: LxmfNodeStatus | null;
  readonly isRunning: boolean;
}) {
  if (nodeStatus) {
    return (
      <>
        <Row label="Mode" value={MODE_LABELS[nodeStatus.mode] || `${nodeStatus.mode}`} />
        <Row label="Identity" value={truncHex(nodeStatus.identityHex, 10)} mono />
        <Row label="LXMF Address" value={nodeStatus.addressHex || '—'} mono />
      </>
    );
  }

  if (isRunning) {
    return <Text style={styles.muted}>Loading identity... (pull to refresh)</Text>;
  }

  return null;
}

function SendResultText({ result }: { readonly result: string | null }) {
  if (!result) {
    return null;
  }
  return <Text style={styles.resultText}>{result}</Text>;
}

const SEED_HEX = 'new';

export default function RootLayout() { // NOSONAR
  const [mode, setMode] = useState<LxmfNodeMode>(LxmfNodeMode.Reticulum);
  const [tcpHost, setTcpHost] = useState('192.168.1.175');
  const [tcpPort, setTcpPort] = useState('4243');
  const [announceMs, setAnnounceMs] = useState('30000');

  const tcpMode = isTcpTransportMode(mode);
  const hostLabel = getTcpHostLabel(mode);
  const modeInfo = getModeInfo(mode, tcpHost, tcpPort);

  const {
    isRunning,
    isNativeAvailable,
    error,
    events,
    status: nodeStatus,
    start,
    stop,
    send,
    getStatus,
    fetchMessages,
  } = useLxmf({
    identityHex: SEED_HEX,
    lxmfAddressHex: SEED_HEX,
    logLevel: 3,
    mode,
    tcpHost: tcpMode ? tcpHost : undefined,
    tcpPort: tcpMode ? Number.parseInt(tcpPort, 10) || 0 : undefined,
    announceIntervalMs: Number.parseInt(announceMs, 10) || 30000,
  });

  const [announces, setAnnounces] = useState<LxmfEvent[]>([]);
  const [messages, setMessages] = useState<LxmfEvent[]>([]);
  const [holdAnnounces, setHoldAnnounces] = useState<HoldAnnounceEntry[]>([]);
  const [missingIdentities, setMissingIdentities] = useState<MissingIdentityEntry[]>([]);
  const [rustLogs, setRustLogs] = useState<string[]>([]);
  const [eventLog, setEventLog] = useState<string[]>([]);
  const [sendDest, setSendDest] = useState('');
  const [sendBody, setSendBody] = useState('Hello from LXMF!');
  const [sendResult, setSendResult] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  useEffect(() => {
    if (events.length === 0) return;

    processIncomingEvents(
      events,
      setEventLog,
      setRustLogs,
      setHoldAnnounces,
      setMissingIdentities,
      setAnnounces,
      setMessages,
    );
  }, [events]);

  const handleStart = useCallback(async () => {
    await start();
    // Status updates automatically via onStatusChanged event → hook's status state
  }, [start]);

  const handleStop = useCallback(async () => {
    await stop();
    // Hook's stop() clears status state
  }, [stop]);

  const handleRefresh = useCallback(() => {
    setRefreshing(true);
    getStatus(); // updates hook's status state as side effect
    fetchMessages(20);
    setRefreshing(false);
  }, [getStatus, fetchMessages]);

  // Deduplicated peers: one entry per destHash, most recent announce wins
  const peers = useMemo(() => {
    const seen = new Set<string>();
    return announces.filter((a) => {
      const dest = String(a.destHash || '').toLowerCase();
      if (!dest || seen.has(dest)) return false;
      seen.add(dest);
      return true;
    });
  }, [announces]);

  const knownDestinations = useMemo(
    () => new Set(announces.map((event) => String(event.destHash || '').toLowerCase())),
    [announces],
  );
  const resolvingDestinations = useMemo(
    () => getResolvingDestinations(holdAnnounces, missingIdentities),
    [holdAnnounces, missingIdentities],
  );

  const handleSend = useCallback(async () => {
    const normalizedDest = sendDest.trim().toLowerCase();
    if (!/^[0-9a-f]{32}$/.test(normalizedDest)) {
      setSendResult('Destination must be 32 hex chars (16 bytes).');
      return;
    }
    if (!knownDestinations.has(normalizedDest)) {
      setSendResult('Destination not announced yet. Wait for a fresh announce, then retry.');
      return;
    }
    if (resolvingDestinations.has(normalizedDest)) {
      setSendResult('Destination is still resolving identity/path. Retry in a few seconds.');
      return;
    }
    await sendMessage(send, normalizedDest, sendBody, setSendResult);
  }, [send, sendDest, sendBody, knownDestinations, resolvingDestinations]);

  const cycleMode = useCallback(() => {
    if (isRunning) return;
    setMode((prev) => getNextMode(prev));
  }, [isRunning]);

  useEffect(() => {
    if (!isRunning) return;
    const interval = setInterval(() => {
      getStatus(); // updates hook's status state as side effect
    }, 5000);
    return () => clearInterval(interval);
  }, [isRunning, getStatus]);

  const rustLogLines = getLineMap(rustLogs.slice(0, 80));
  const eventLogLines = getLineMap(eventLog.slice(0, 30));

  return (
    <ScrollView
      style={styles.container}
      refreshControl={<RefreshControl refreshing={refreshing} onRefresh={handleRefresh} />}
    >
      <View style={styles.content}>
        <Text style={styles.title}>LXMF React Native</Text>
        <Text style={styles.subtitle}>Reticulum mesh networking via rns-transport (Rust)</Text>

        <View style={styles.card}>
          <Text style={styles.cardTitle}>Node Identity</Text>
          <Row label="Native module" value={isNativeAvailable ? 'Loaded' : 'Missing'} />
          <Row
            label="Status"
            value={isRunning ? 'Running' : 'Stopped'}
            valueColor={isRunning ? '#3fb950' : '#f85149'}
          />
          <NodeStatusDetails nodeStatus={nodeStatus} isRunning={isRunning} />
        </View>

        <ErrorBanner error={error} />

        <View style={styles.card}>
          <Text style={styles.cardTitle}>Interface</Text>

          <View style={styles.modeRow}>
            <Text style={styles.rowLabel}>Transport</Text>
            <TouchableOpacity
              style={[styles.modeBadge, isRunning && styles.btnDisabled]}
              onPress={cycleMode}
              disabled={isRunning}
              activeOpacity={0.7}
            >
              <Text style={styles.modeBadgeText}>{MODE_LABELS[mode]}</Text>
            </TouchableOpacity>
          </View>

          {tcpMode && (
            <View style={styles.tcpFields}>
              <Text style={styles.fieldLabel}>{hostLabel}</Text>
              <TextInput
                style={styles.input}
                placeholder="192.168.1.175"
                placeholderTextColor="#484f58"
                value={tcpHost}
                onChangeText={setTcpHost}
                editable={!isRunning}
                autoCapitalize="none"
                autoCorrect={false}
              />
              <Text style={styles.fieldLabel}>Port</Text>
              <TextInput
                style={styles.input}
                placeholder="4243"
                placeholderTextColor="#484f58"
                value={tcpPort}
                onChangeText={setTcpPort}
                editable={!isRunning}
                keyboardType="numeric"
              />
            </View>
          )}

          <Text style={styles.fieldLabel}>Announce interval (ms)</Text>
          <TextInput
            style={styles.input}
            placeholder="30000"
            placeholderTextColor="#484f58"
            value={announceMs}
            onChangeText={setAnnounceMs}
            editable={!isRunning}
            keyboardType="numeric"
          />

          <View style={styles.infoBox}>
            <Text style={styles.infoText}>{modeInfo}</Text>
          </View>
        </View>

        <View style={styles.card}>
          <Text style={styles.cardTitle}>Controls</Text>
          <View style={styles.buttonRow}>
            <Btn
              label={isRunning ? 'Running' : 'Start'}
              onPress={handleStart}
              color={isRunning ? '#238636' : '#1f6feb'}
              disabled={isRunning}
            />
            <Btn label="Stop" onPress={handleStop} color="#da3633" disabled={!isRunning} />
            <Btn label="Refresh" onPress={handleRefresh} color="#d29922" />
          </View>
        </View>

        <View style={styles.card}>
          <Text style={styles.cardTitle}>Send Message</Text>
          <TextInput
            style={styles.input}
            placeholder="Destination address hex (32 chars)"
            placeholderTextColor="#484f58"
            value={sendDest}
            onChangeText={setSendDest}
            autoCapitalize="none"
            autoCorrect={false}
          />
          <TextInput
            style={styles.input}
            placeholder="Message body"
            placeholderTextColor="#484f58"
            value={sendBody}
            onChangeText={setSendBody}
          />
          <Btn label="Send" onPress={handleSend} color="#1f6feb" disabled={!isRunning} />
          <SendResultText result={sendResult} />
        </View>

        <View style={styles.card}>
          <Text style={styles.cardTitle}>Peers ({peers.length})</Text>
          {peers.length > 0 ? (
            peers.slice(0, 40).map((peer) => {
              const destination = String(peer.destHash || '').toLowerCase();
              const name = peer.appData ? String(peer.appData).trim() : null;
              const hops = peer.hops ?? '?';
              const sendReady = destination.length === 32 && !resolvingDestinations.has(destination);

              return (
                <TouchableOpacity
                  key={destination}
                  style={styles.peerRow}
                  onPress={() => setSendDest(destination)}
                  activeOpacity={0.75}
                >
                  <View style={styles.peerAvatar}>
                    <Text style={styles.peerAvatarText}>
                      {name ? name.slice(0, 1).toUpperCase() : '#'}
                    </Text>
                  </View>
                  <View style={styles.peerInfo}>
                    <View style={styles.peerNameRow}>
                      <Text style={styles.peerName} numberOfLines={1}>
                        {name || truncHex(destination, 10)}
                      </Text>
                      <Text style={styles.peerHops}>{hops} hop{hops !== 1 ? 's' : ''}</Text>
                    </View>
                    <View style={styles.peerAddressRow}>
                      <Text style={styles.peerAddress}>{truncHex(destination, 10)}</Text>
                      <Text style={sendReady ? styles.sendReadyBadge : styles.sendResolvingBadge}>
                        {sendReady ? 'SEND READY' : 'RESOLVING'}
                      </Text>
                    </View>
                  </View>
                </TouchableOpacity>
              );
            })
          ) : (
            <Text style={styles.muted}>
              {isRunning
                ? 'Listening for peers on the network...'
                : 'Start the node to discover peers'}
            </Text>
          )}
        </View>

        <View style={styles.card}>
          <Text style={styles.cardTitle}>Messages ({messages.length})</Text>
          {messages.length > 0 ? (
            messages.slice(0, 30).map((message, idx) => {
              const rawContent = String(message.content || '');
              const contentBytes = rawContent.length > 0 && rawContent.length % 2 === 0
                ? hexToBytes(rawContent) : null;
              // Real sender is in wire header bytes 16-31; fallback to event source field
              const realSource = contentBytes ? lxmfSourceHex(contentBytes) : null;
              const displaySource = realSource ?? String(message.source || '???');
              return (
                <View key={`${getMessageKey(message)}_${idx}`} style={styles.listItem}>
                  <Text style={styles.mono}>{truncHex(displaySource, 12)}</Text>
                  <Text style={styles.messageText} numberOfLines={4}>
                    {decodeLxmfContent(rawContent)}
                  </Text>
                </View>
              );
            })
          ) : (
            <Text style={styles.muted}>No messages yet</Text>
          )}
        </View>

        <View style={styles.card}>
          <Text style={styles.cardTitle}>Transport Hold Queue ({holdAnnounces.length})</Text>
          {holdAnnounces.length > 0 ? (
            holdAnnounces.slice(0, 20).map((entry) => (
              <View key={entry.key} style={styles.listItem}>
                <Text style={styles.mono}>dest {truncHex(entry.destination, 10)}</Text>
                <Text style={styles.mono}>iface {truncHex(entry.iface, 10)}</Text>
                <Text style={styles.muted}>held {entry.holdSeconds}s at {entry.at}</Text>
              </View>
            ))
          ) : (
            <Text style={styles.muted}>Parsed hold-announce rows will appear here</Text>
          )}
        </View>

        <View style={styles.card}>
          <Text style={styles.cardTitle}>Missing Identity Warnings ({missingIdentities.length})</Text>
          {missingIdentities.length > 0 ? (
            missingIdentities.slice(0, 20).map((entry) => (
              <View key={entry.key} style={styles.listItem}>
                <Text style={styles.mono}>dest {truncHex(entry.destination, 10)}</Text>
                <Text style={styles.muted}>missing destination identity at {entry.at}</Text>
              </View>
            ))
          ) : (
            <Text style={styles.muted}>Missing identity warnings will appear here</Text>
          )}
        </View>

        <View style={styles.card}>
          <Text style={styles.cardTitle}>Rust Logs ({rustLogs.length})</Text>
          {rustLogLines.length > 0 ? (
            rustLogLines.map((entry) => (
              <Text key={entry.key} style={styles.logLine} selectable>
                {entry.line}
              </Text>
            ))
          ) : (
            <Text style={styles.muted}>Rust logs will appear here when the node is running</Text>
          )}
        </View>

        <View style={styles.card}>
          <Text style={styles.cardTitle}>Event Log ({eventLog.length})</Text>
          {eventLogLines.length > 0 ? (
            eventLogLines.map((entry) => (
              <Text key={entry.key} style={styles.logLine} numberOfLines={3}>
                {entry.line}
              </Text>
            ))
          ) : (
            <Text style={styles.muted}>Events will appear here</Text>
          )}
        </View>

        <View style={{ height: 60 }} />
      </View>
    </ScrollView>
  );
}

function Row({
  label,
  value,
  mono,
  valueColor,
}: Readonly<{
  label: string;
  value: string;
  mono?: boolean;
  valueColor?: string;
}>) {
  return (
    <View style={styles.row}>
      <Text style={styles.rowLabel}>{label}</Text>
      <Text style={[styles.rowValue, mono && styles.mono, valueColor ? { color: valueColor } : null]}>
        {value}
      </Text>
    </View>
  );
}

function Btn({
  label,
  onPress,
  color,
  disabled,
}: Readonly<{
  label: string;
  onPress: () => void;
  color: string;
  disabled?: boolean;
}>) {
  return (
    <TouchableOpacity
      style={[styles.btn, { backgroundColor: color }, disabled && styles.btnDisabled]}
      onPress={onPress}
      disabled={disabled}
      activeOpacity={0.7}
    >
      <Text style={styles.btnText}>{label}</Text>
    </TouchableOpacity>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1, backgroundColor: '#0d1117' },
  content: { paddingHorizontal: 16, paddingTop: 50 },
  title: { fontSize: 26, fontWeight: 'bold', color: '#e6edf3' },
  subtitle: { fontSize: 13, color: '#7d8590', marginBottom: 20 },
  card: {
    backgroundColor: '#161b22',
    borderRadius: 12,
    padding: 16,
    marginBottom: 12,
    borderWidth: 1,
    borderColor: '#30363d',
  },
  cardTitle: { fontSize: 16, fontWeight: '600', color: '#e6edf3', marginBottom: 10 },
  row: { flexDirection: 'row', justifyContent: 'space-between', paddingVertical: 4 },
  rowLabel: { color: '#7d8590', fontSize: 14 },
  rowValue: { color: '#e6edf3', fontSize: 14 },
  mono: { fontFamily: 'monospace', fontSize: 13, color: '#79c0ff' },
  muted: { color: '#484f58', fontSize: 13, fontStyle: 'italic' },
  errorCard: {
    backgroundColor: '#3d1114',
    borderColor: '#f8514966',
    borderWidth: 1,
    borderRadius: 12,
    padding: 14,
    marginBottom: 12,
  },
  errorText: { color: '#f85149', fontSize: 14 },
  modeRow: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    alignItems: 'center',
    marginBottom: 12,
  },
  modeBadge: {
    backgroundColor: '#1f6feb',
    paddingHorizontal: 14,
    paddingVertical: 6,
    borderRadius: 16,
  },
  modeBadgeText: { color: '#fff', fontWeight: '600', fontSize: 13 },
  tcpFields: { marginBottom: 8 },
  fieldLabel: { color: '#7d8590', fontSize: 12, marginBottom: 4, marginTop: 4 },
  infoBox: {
    backgroundColor: '#0d1117',
    borderWidth: 1,
    borderColor: '#1f6feb44',
    borderRadius: 8,
    padding: 10,
    marginTop: 8,
  },
  infoText: { color: '#58a6ff', fontSize: 12, lineHeight: 18 },
  buttonRow: { flexDirection: 'row', gap: 8, marginBottom: 8 },
  btn: {
    paddingHorizontal: 14,
    paddingVertical: 10,
    borderRadius: 8,
    flex: 1,
    alignItems: 'center',
  },
  btnDisabled: { opacity: 0.4 },
  btnText: { color: '#fff', fontWeight: '600', fontSize: 14 },
  input: {
    backgroundColor: '#0d1117',
    borderWidth: 1,
    borderColor: '#30363d',
    borderRadius: 8,
    padding: 10,
    color: '#e6edf3',
    fontSize: 14,
    fontFamily: 'monospace',
    marginBottom: 8,
  },
  resultText: { color: '#3fb950', fontSize: 13, marginTop: 6 },
  listItem: {
    paddingVertical: 6,
    borderBottomWidth: 1,
    borderBottomColor: '#21262d',
  },
  announceHeaderRow: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    gap: 8,
  },
  sendReadyBadge: {
    color: '#3fb950',
    fontSize: 11,
    fontWeight: '700',
  },
  sendResolvingBadge: {
    color: '#d29922',
    fontSize: 11,
    fontWeight: '700',
  },
  messageText: {
    color: '#e6edf3',
    fontSize: 13,
    lineHeight: 18,
    marginTop: 2,
  },
  peerRow: {
    flexDirection: 'row',
    alignItems: 'center',
    paddingVertical: 8,
    borderBottomWidth: 1,
    borderBottomColor: '#21262d',
    gap: 12,
  },
  peerAvatar: {
    width: 38,
    height: 38,
    borderRadius: 19,
    backgroundColor: '#1f6feb33',
    borderWidth: 1,
    borderColor: '#1f6feb66',
    alignItems: 'center',
    justifyContent: 'center',
    flexShrink: 0,
  },
  peerAvatarText: {
    color: '#58a6ff',
    fontSize: 16,
    fontWeight: '700',
  },
  peerInfo: {
    flex: 1,
    gap: 2,
  },
  peerNameRow: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    gap: 8,
  },
  peerName: {
    color: '#e6edf3',
    fontSize: 14,
    fontWeight: '600',
    flex: 1,
  },
  peerHops: {
    color: '#7d8590',
    fontSize: 12,
    flexShrink: 0,
  },
  peerAddressRow: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    gap: 8,
  },
  peerAddress: {
    color: '#79c0ff',
    fontSize: 11,
    fontFamily: 'monospace',
    flex: 1,
  },
  logLine: {
    color: '#7d8590',
    fontSize: 11,
    fontFamily: 'monospace',
    paddingVertical: 2,
  },
});
