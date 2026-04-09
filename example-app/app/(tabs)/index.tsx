import { useCallback, useMemo, useState } from 'react';
import {
  PermissionsAndroid,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from 'react-native';
import { LxmfNodeMode, type LxmfEvent, useLxmf } from '@lxmf/react-native';

// ── Helpers ──────────────────────────────────────────────────────────────────

const B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';

function bytesToBase64(bytes: Uint8Array): string {
  let out = '';
  for (let i = 0; i < bytes.length; i += 3) {
    const b0 = bytes[i] ?? 0, b1 = bytes[i + 1] ?? 0, b2 = bytes[i + 2] ?? 0;
    const t = (b0 << 16) | (b1 << 8) | b2;
    out += B64[(t >> 18) & 0x3f];
    out += B64[(t >> 12) & 0x3f];
    out += i + 1 < bytes.length ? B64[(t >> 6) & 0x3f] : '=';
    out += i + 2 < bytes.length ? B64[t & 0x3f] : '=';
  }
  return out;
}

function utf8ToBase64(s: string): string {
  if (typeof globalThis.btoa === 'function') return globalThis.btoa(s);
  if (typeof TextEncoder !== 'undefined') return bytesToBase64(new TextEncoder().encode(s));
  return s;
}

function shortHex(v: string): string {
  if (!v) return '—';
  return v.length <= 12 ? v : `${v.slice(0, 6)}…${v.slice(-6)}`;
}

function ts(e: LxmfEvent): number | null {
  const r = e.timestamp ?? e.ts ?? e.time ?? e.epoch;
  if (typeof r === 'number' && Number.isFinite(r)) return r;
  if (typeof r === 'string') { const n = Number(r); return Number.isFinite(n) ? n : null; }
  return null;
}

function fmtTime(e: LxmfEvent): string {
  const t = ts(e);
  if (!t) return 'now';
  return new Date(t > 10_000_000_000 ? t : t * 1000)
    .toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
}

function evtSummary(e: LxmfEvent): string {
  if (e.type === 'announceReceived') {
    const from = shortHex(String(e.destHash ?? e.address ?? e.source ?? '?'));
    const hops = e.hops ?? e.hopCount;
    return hops === undefined ? `Announce ${from}` : `Announce ${from} (${hops} hop)`;
  }
  if (e.type === 'messageReceived') return `Msg from ${shortHex(String(e.source ?? e.from ?? '?'))}`;
  if (e.type === 'log') return String(e.message ?? e.msg ?? 'log');
  if (e.type === 'error') return String(e.message ?? 'error');
  return e.type;
}

function evtKey(e: LxmfEvent, prefix = ''): string {
  const t = ts(e) ?? 'na';
  const m = String(e.id ?? e.receipt ?? e.destHash ?? e.source ?? e.message ?? 'ev');
  return `${prefix}${e.type}-${t}-${m}`;
}

// ── Accordion ─────────────────────────────────────────────────────────────────

function Accordion({
  title,
  badge,
  defaultOpen = false,
  children,
}: Readonly<{
  title: string;
  badge?: string | number;
  defaultOpen?: boolean;
  children: React.ReactNode;
}>) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <View style={S.accordion}>
      <Pressable
        style={({ pressed }) => [S.accordionHeader, pressed && S.accordionHeaderPressed]}
        onPress={() => setOpen(o => !o)}>
        <Text style={S.accordionChevron}>{open ? '▾' : '▸'}</Text>
        <Text style={S.accordionTitle}>{title}</Text>
        {badge === undefined ? null : (
          <View style={S.accordionBadge}>
            <Text style={S.accordionBadgeText}>{badge}</Text>
          </View>
        )}
      </Pressable>
      {open ? <View style={S.accordionBody}>{children}</View> : null}
    </View>
  );
}

// ── Tiny components ──────────────────────────────────────────────────────────

function Btn({
  label, onPress, disabled, danger, small,
}: Readonly<{ label: string; onPress: () => void; disabled?: boolean; danger?: boolean; small?: boolean }>) {
  return (
    <Pressable
      style={({ pressed }) => [
        S.btn, danger && S.btnDanger, disabled && S.btnDisabled, small && S.btnSmall,
        pressed && !disabled && S.btnPressed,
      ]}
      onPress={onPress}
      disabled={disabled}>
      <Text style={[S.btnText, small && S.btnTextSmall]}>{label}</Text>
    </Pressable>
  );
}

function Row({ label, value }: Readonly<{ label: string; value: string }>) {
  return (
    <View style={S.statRow}>
      <Text style={S.statLabel}>{label}</Text>
      <Text selectable style={S.statValue}>{value}</Text>
    </View>
  );
}

function Pill({ label, active }: Readonly<{ label: string; active: boolean }>) {
  return (
    <View style={[S.pill, active && S.pillActive]}>
      <Text style={[S.pillText, active && S.pillTextActive]}>{label}</Text>
    </View>
  );
}

// ── Main screen ──────────────────────────────────────────────────────────────

const SEED = 'new';

export default function HomeScreen() {
  // Transport state
  const [tcpHost, setTcpHost] = useState('127.0.0.1');
  const [tcpPort, setTcpPort] = useState('4242');
  const [bleActive, setBleActive] = useState(false);
  const [tcpActive, setTcpActive] = useState(false);
  const [transportMsg, setTransportMsg] = useState('');

  // Send state
  const [dest, setDest] = useState('');
  const [msgText, setMsgText] = useState('Hello from LXMF');
  const [sendResult, setSendResult] = useState('');

  const {
    isNativeAvailable, isRunning, status, error, events, beacons,
    start, stop, send, getStatus, startBLE, stopBLE,
  } = useLxmf({ identityHex: SEED, lxmfAddressHex: SEED, logLevel: 3 });

  // ── Derived ───────────────────────────────────────────────────────────────

  const counts = useMemo(() => {
    let announces = 0, logs = 0, messages = 0, errors = 0;
    for (const e of events) {
      if (e.type === 'announceReceived') announces++;
      if (e.type === 'log') logs++;
      if (e.type === 'messageReceived') messages++;
      if (e.type === 'error') errors++;
    }
    return { announces, logs, messages, errors };
  }, [events]);

  const announceEvts = useMemo(() => events.filter(e => e.type === 'announceReceived').slice(0, 20), [events]);
  const msgEvts = useMemo(() => events.filter(e => e.type === 'messageReceived').slice(0, 20), [events]);
  const logEvts = useMemo(() => events.filter(e => e.type === 'log').slice(0, 30), [events]);
  const allEvts = useMemo(() => events.slice(0, 30), [events]);

  // ── Actions ───────────────────────────────────────────────────────────────

  const onStartTcp = useCallback(async () => {
    setTransportMsg('');
    const host = tcpHost.trim();
    const port = Number(tcpPort);
    if (!host) { setTransportMsg('Host required.'); return; }
    if (!Number.isInteger(port) || port < 1 || port > 65535) { setTransportMsg('Port 1–65535.'); return; }
    const ok = await start({ mode: LxmfNodeMode.Reticulum, tcpInterfaces: [{ host, port }] });
    if (ok) setTcpActive(true);
  }, [tcpHost, tcpPort, start]);

  const onStopTcp = useCallback(async () => {
    await stop();
    setTcpActive(false);
  }, [stop]);

  const onStartBle = useCallback(async () => {
    if (Platform.OS === 'android') {
      const perms = Platform.Version >= 31
        ? [
            PermissionsAndroid.PERMISSIONS.BLUETOOTH_SCAN,
            PermissionsAndroid.PERMISSIONS.BLUETOOTH_ADVERTISE,
            PermissionsAndroid.PERMISSIONS.BLUETOOTH_CONNECT,
          ]
        : [PermissionsAndroid.PERMISSIONS.ACCESS_FINE_LOCATION];
      const results = await PermissionsAndroid.requestMultiple(perms);
      const denied = Object.values(results).some(r => r !== PermissionsAndroid.RESULTS.GRANTED);
      if (denied) {
        setTransportMsg('BLE permissions denied.');
        return;
      }
    }
    startBLE();
    setBleActive(true);
  }, [startBLE]);

  const onStopBle = useCallback(() => {
    stopBLE();
    setBleActive(false);
  }, [stopBLE]);

  const onSend = useCallback(async () => {
    const d = dest.trim().toLowerCase();
    if (!/^[0-9a-f]{32}$/.test(d)) { setSendResult('Dest = 32 hex chars.'); return; }
    const r = await send(d, utf8ToBase64(msgText));
    setSendResult(r >= 0 ? `Receipt #${r}` : 'Send failed.');
  }, [dest, msgText, send]);

  // ── Render ────────────────────────────────────────────────────────────────

  return (
    <ScrollView contentContainerStyle={S.scroll} contentInsetAdjustmentBehavior="automatic">

      {/* Header */}
      <View style={S.header}>
        <Text style={S.headerTitle}>LXMF Console</Text>
        <View style={S.headerPills}>
          <Pill label="BLE" active={bleActive} />
          <Pill label="TCP" active={tcpActive} />
          <Pill label={isRunning ? 'Running' : 'Stopped'} active={isRunning} />
        </View>
      </View>

      {/* Error banner */}
      {error ? (
        <View style={S.errorBanner}>
          <Text style={S.errorBannerText}>{error}</Text>
        </View>
      ) : null}

      {/* ── Node Status ─────────────────────────────────────────────────── */}
      <Accordion title="Node Status" defaultOpen>
        <Row label="Native module" value={isNativeAvailable ? 'Loaded ✓' : 'Missing ✗'} />
        <Row label="State" value={isRunning ? 'Running' : 'Stopped'} />
        <Row label="Identity" value={shortHex(status?.identityHex ?? '')} />
        <Row label="Address" value={shortHex(status?.addressHex ?? '')} />
        <Row label="Announces" value={String(status?.announcesReceived ?? 0)} />
        <Row label="Messages" value={String(status?.lxmfMessagesReceived ?? 0)} />
        <Row label="Outbound sent" value={String(status?.outboundSent ?? 0)} />
        <Row label="Inbound accepted" value={String(status?.inboundAccepted ?? 0)} />
        <View style={S.btnRow}>
          <Btn label="Refresh" onPress={getStatus} small />
        </View>
      </Accordion>

      {/* ── TCP / Reticulum ──────────────────────────────────────────────── */}
      <Accordion title="TCP / Reticulum" defaultOpen>
        <Text style={S.hint}>Connect to rnsd daemon. BLE can run simultaneously.</Text>
        <TextInput
          style={S.input}
          placeholder="Host (e.g. 192.168.1.10)"
          placeholderTextColor="#607080"
          value={tcpHost}
          onChangeText={setTcpHost}
          autoCapitalize="none"
          autoCorrect={false}
        />
        <TextInput
          style={S.input}
          placeholder="Port (default 4242)"
          placeholderTextColor="#607080"
          value={tcpPort}
          onChangeText={setTcpPort}
          keyboardType="number-pad"
        />
        {transportMsg ? <Text style={S.warn}>{transportMsg}</Text> : null}
        <View style={S.btnRow}>
          <Btn label="Start TCP" onPress={onStartTcp} disabled={!isNativeAvailable || isRunning} />
          <Btn label="Stop TCP" onPress={onStopTcp} disabled={!isRunning} danger />
        </View>
      </Accordion>

      {/* ── BLE Mesh ─────────────────────────────────────────────────────── */}
      <Accordion title="BLE Mesh" defaultOpen>
        <Text style={S.hint}>BLE scan + advertise independent of TCP. Both can run together.</Text>
        <Row label="BLE active" value={bleActive ? 'Yes' : 'No'} />
        <Row label="BLE peers" value={String(status?.blePeerCount ?? 0)} />
        <View style={S.btnRow}>
          <Btn label="Start BLE" onPress={onStartBle} disabled={bleActive} />
          <Btn label="Stop BLE" onPress={onStopBle} disabled={!bleActive} danger />
        </View>
      </Accordion>

      {/* ── Send Message ─────────────────────────────────────────────────── */}
      <Accordion title="Send Message" defaultOpen>
        <TextInput
          style={S.input}
          placeholder="Destination (32 hex chars)"
          placeholderTextColor="#607080"
          value={dest}
          onChangeText={setDest}
          autoCapitalize="none"
          autoCorrect={false}
        />
        <TextInput
          style={S.input}
          placeholder="Message text"
          placeholderTextColor="#607080"
          value={msgText}
          onChangeText={setMsgText}
        />
        <View style={S.btnRow}>
          <Btn label="Send" onPress={onSend} disabled={!isRunning} />
        </View>
        {sendResult ? <Text style={S.feedback}>{sendResult}</Text> : null}
      </Accordion>

      {/* ── Peers / Beacons ──────────────────────────────────────────────── */}
      <Accordion title="Peers / Beacons" badge={beacons.length} defaultOpen={false}>
        {beacons.length === 0 ? (
          <Text style={S.muted}>No peers yet.</Text>
        ) : (
          beacons.map((b, i) => (
            <View key={`${b.destHash}-${i}`} style={S.itemCard}>
              <Text selectable style={S.itemTitle}>{shortHex(b.destHash)}</Text>
              <Text style={S.itemMeta}>state: {b.state} · reconnects: {b.reconnectAttempts}</Text>
            </View>
          ))
        )}
      </Accordion>

      {/* ── Announces ────────────────────────────────────────────────────── */}
      <Accordion title="Announces" badge={counts.announces} defaultOpen={false}>
        {announceEvts.length === 0 ? (
          <Text style={S.muted}>No announces yet.</Text>
        ) : (
          announceEvts.map((e: LxmfEvent, i: number) => (
            <View key={`${evtKey(e, 'ann-')}-${i}`} style={S.itemCard}>
              <Text selectable style={S.itemTitle}>{evtSummary(e)}</Text>
              <Text style={S.itemMeta}>{fmtTime(e)}</Text>
            </View>
          ))
        )}
      </Accordion>

      {/* ── Messages ─────────────────────────────────────────────────────── */}
      <Accordion title="Messages" badge={counts.messages} defaultOpen={false}>
        {msgEvts.length === 0 ? (
          <Text style={S.muted}>No messages yet.</Text>
        ) : (
          msgEvts.map(e => (
            <View key={evtKey(e, 'msg-')} style={S.itemCard}>
              <Text selectable style={S.itemTitle}>{evtSummary(e)}</Text>
              {e.content ? (
                <Text selectable style={S.itemBody}>{String(e.content)}</Text>
              ) : null}
              <Text style={S.itemMeta}>{fmtTime(e)}</Text>
            </View>
          ))
        )}
      </Accordion>

      {/* ── Event Log ────────────────────────────────────────────────────── */}
      <Accordion title="Event Log" badge={allEvts.length} defaultOpen={false}>
        {allEvts.length === 0 ? (
          <Text style={S.muted}>No events yet.</Text>
        ) : (
          allEvts.map(e => (
            <View key={evtKey(e, 'el-')} style={S.logRow}>
              <Text style={S.logTag}>{e.type}</Text>
              <Text selectable style={S.logText} numberOfLines={2}>{evtSummary(e)}</Text>
              <Text style={S.logTime}>{fmtTime(e)}</Text>
            </View>
          ))
        )}
      </Accordion>

      {/* ── Debug Logs ───────────────────────────────────────────────────── */}
      <Accordion title="Debug Logs" badge={counts.logs} defaultOpen={false}>
        {logEvts.length === 0 ? (
          <Text style={S.muted}>No logs yet.</Text>
        ) : (
          logEvts.map(e => (
            <Text key={evtKey(e, 'lg-')} selectable style={S.logLine} numberOfLines={3}>
              [{fmtTime(e)}] {evtSummary(e)}
            </Text>
          ))
        )}
      </Accordion>

    </ScrollView>
  );
}

// ── Styles ───────────────────────────────────────────────────────────────────

const C = {
  bg: '#0c1218',
  surface: '#131d26',
  border: '#1e3040',
  accent: '#1a7fc1',
  accentBright: '#4fb3e8',
  danger: '#c0392b',
  text: '#d8ecf8',
  textDim: '#7a9db5',
  textMono: '#a8c8dc',
  green: '#2ecc71',
  warn: '#f0a500',
};

const S = StyleSheet.create({
  scroll: {
    paddingHorizontal: 14,
    paddingTop: 14,
    paddingBottom: 60,
    gap: 10,
    backgroundColor: C.bg,
  },

  // Header
  header: {
    backgroundColor: C.surface,
    borderRadius: 14,
    borderWidth: 1,
    borderColor: C.border,
    padding: 14,
    gap: 10,
  },
  headerTitle: { color: C.text, fontSize: 26, fontWeight: '700' },
  headerPills: { flexDirection: 'row', gap: 8 },

  pill: {
    borderRadius: 20,
    borderWidth: 1,
    borderColor: C.border,
    paddingHorizontal: 10,
    paddingVertical: 4,
    backgroundColor: '#0e1923',
  },
  pillActive: { borderColor: C.accentBright, backgroundColor: '#0d3550' },
  pillText: { color: C.textDim, fontSize: 12, fontWeight: '600' },
  pillTextActive: { color: C.accentBright },

  errorBanner: {
    backgroundColor: '#3a1515',
    borderRadius: 10,
    borderWidth: 1,
    borderColor: '#7a2020',
    padding: 10,
  },
  errorBannerText: { color: '#ff9a9a', fontSize: 13 },

  // Accordion
  accordion: {
    backgroundColor: C.surface,
    borderRadius: 14,
    borderWidth: 1,
    borderColor: C.border,
    overflow: 'hidden',
  },
  accordionHeader: {
    flexDirection: 'row',
    alignItems: 'center',
    paddingHorizontal: 14,
    paddingVertical: 13,
    gap: 8,
  },
  accordionHeaderPressed: { backgroundColor: '#17232e' },
  accordionChevron: { color: C.textDim, fontSize: 14, width: 14 },
  accordionTitle: { color: C.text, fontSize: 16, fontWeight: '600', flex: 1 },
  accordionBadge: {
    backgroundColor: '#0d3550',
    borderRadius: 10,
    paddingHorizontal: 7,
    paddingVertical: 2,
    minWidth: 24,
    alignItems: 'center',
  },
  accordionBadgeText: { color: C.accentBright, fontSize: 11, fontWeight: '700' },
  accordionBody: {
    paddingHorizontal: 14,
    paddingBottom: 14,
    gap: 8,
    borderTopWidth: 1,
    borderTopColor: C.border,
  },

  // Stat rows
  statRow: { flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center' },
  statLabel: { color: C.textDim, fontSize: 13 },
  statValue: { color: C.text, fontSize: 13, fontFamily: 'monospace' },

  hint: { color: C.textDim, fontSize: 12, marginBottom: 2 },

  // Input
  input: {
    borderWidth: 1,
    borderColor: '#2a4050',
    backgroundColor: '#0b1820',
    color: C.text,
    borderRadius: 10,
    paddingHorizontal: 10,
    paddingVertical: 10,
    fontFamily: 'monospace',
    fontSize: 13,
  },

  warn: { color: C.warn, fontSize: 12, fontFamily: 'monospace' },
  feedback: { color: C.green, fontSize: 13, fontFamily: 'monospace' },
  muted: { color: C.textDim, fontSize: 13 },

  // Buttons
  btnRow: { flexDirection: 'row', gap: 8, marginTop: 2 },
  btn: {
    flex: 1,
    borderRadius: 10,
    paddingVertical: 10,
    alignItems: 'center',
    backgroundColor: C.accent,
  },
  btnSmall: { paddingVertical: 7, flex: 0, paddingHorizontal: 16 },
  btnDanger: { backgroundColor: C.danger },
  btnDisabled: { opacity: 0.4 },
  btnPressed: { opacity: 0.78 },
  btnText: { color: '#e8f6ff', fontSize: 14, fontWeight: '600' },
  btnTextSmall: { fontSize: 12 },

  // Item cards (announces, messages, beacons)
  itemCard: {
    borderWidth: 1,
    borderColor: '#1f3348',
    borderRadius: 10,
    padding: 10,
    backgroundColor: '#0e1e2b',
    gap: 3,
  },
  itemTitle: { color: C.text, fontSize: 13, fontWeight: '600' },
  itemBody: { color: C.textMono, fontSize: 13, fontFamily: 'monospace' },
  itemMeta: { color: C.textDim, fontSize: 11, fontFamily: 'monospace' },

  // Log rows
  logRow: { flexDirection: 'row', alignItems: 'flex-start', gap: 6 },
  logTag: { color: C.accentBright, fontFamily: 'monospace', fontSize: 10, width: 100 },
  logText: { color: C.textMono, flex: 1, fontSize: 11, fontFamily: 'monospace' },
  logTime: { color: C.textDim, fontFamily: 'monospace', fontSize: 10 },
  logLine: { color: C.textMono, fontSize: 11, fontFamily: 'monospace' },
});
