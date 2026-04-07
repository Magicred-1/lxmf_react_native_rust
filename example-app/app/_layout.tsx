import React, { useEffect, useState, useCallback } from 'react';
import {
  View,
  Text,
  ScrollView,
  StyleSheet,
  TouchableOpacity,
  TextInput,
  RefreshControl,
} from 'react-native';
import { useLxmf, LxmfNodeMode, LxmfNodeStatus, Beacon, LxmfEvent } from '@lxmf/react-native';

// Generate a random hex string of `bytes` length
function randomHex(bytes: number): string {
  const arr = new Uint8Array(bytes);
  for (let i = 0; i < bytes; i++) {
    arr[i] = Math.floor(Math.random() * 256);
  }
  return Array.from(arr, (b) => b.toString(16).padStart(2, '0')).join('');
}

function truncHex(hex: string, len = 8): string {
  return hex.length > len * 2 ? hex.slice(0, len) + '...' + hex.slice(-len) : hex;
}

const MODE_LABELS: Record<LxmfNodeMode, string> = {
  [LxmfNodeMode.BleOnly]: 'BLE Only',
  [LxmfNodeMode.TcpClient]: 'TCP Client (FFI)',
  [LxmfNodeMode.TcpServer]: 'TCP Server (FFI)',
  [LxmfNodeMode.Reticulum]: 'Reticulum TCP',
};

// Stable random identity for this app session
const DEV_IDENTITY_HEX = randomHex(32);
const DEV_ADDRESS_HEX = randomHex(16);

export default function RootLayout() {
  // Interface config (set before starting)
  const [mode, setMode] = useState<LxmfNodeMode>(LxmfNodeMode.BleOnly);
  const [tcpHost, setTcpHost] = useState('127.0.0.1');
  const [tcpPort, setTcpPort] = useState('4242');
  const [announceMs, setAnnounceMs] = useState('5000');

  const {
    isRunning,
    isNativeAvailable,
    error,
    events,
    start,
    stop,
    send,
    getStatus,
    getBeacons,
    fetchMessages,
    setLogLevel,
  } = useLxmf({
    identityHex: DEV_IDENTITY_HEX,
    lxmfAddressHex: DEV_ADDRESS_HEX,
    logLevel: 3,
    mode,
    tcpHost: mode !== LxmfNodeMode.BleOnly ? tcpHost : undefined,
    tcpPort: mode !== LxmfNodeMode.BleOnly ? parseInt(tcpPort, 10) || 0 : undefined,
    announceIntervalMs: parseInt(announceMs, 10) || 5000,
  });

  const [nodeStatus, setNodeStatus] = useState<LxmfNodeStatus | null>(null);
  const [discoveredBeacons, setDiscoveredBeacons] = useState<Beacon[]>([]);
  const [messages, setMessages] = useState<any[]>([]);
  const [eventLog, setEventLog] = useState<string[]>([]);
  const [sendDest, setSendDest] = useState('');
  const [sendBody, setSendBody] = useState('Hello from LXMF!');
  const [sendResult, setSendResult] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  // Track events in the log
  useEffect(() => {
    if (events.length > 0) {
      const newEntries = events.map(
        (e: LxmfEvent) =>
          `[${new Date().toLocaleTimeString()}] ${e.type}: ${JSON.stringify(e).slice(0, 120)}`
      );
      setEventLog((prev) => [...newEntries, ...prev].slice(0, 50));
    }
  }, [events]);

  const handleStart = useCallback(async () => {
    await start();
  }, [start]);

  const handleStop = useCallback(async () => {
    await stop();
  }, [stop]);

  const handleRefresh = useCallback(() => {
    setRefreshing(true);
    const s = getStatus();
    if (s) setNodeStatus(s);
    const b = getBeacons();
    if (b) setDiscoveredBeacons(b);
    const m = fetchMessages(20);
    if (m) setMessages(m);
    setRefreshing(false);
  }, [getStatus, getBeacons, fetchMessages]);

  const handleSend = useCallback(async () => {
    if (!sendDest) {
      setSendResult('Enter a destination hex');
      return;
    }
    const bodyB64 = btoa(sendBody);
    const receipt = await send(sendDest, bodyB64);
    if (receipt >= 0) {
      setSendResult(`Sent! receipt #${receipt}`);
    } else {
      setSendResult('Send failed');
    }
  }, [send, sendDest, sendBody]);

  const cycleMode = useCallback(() => {
    if (isRunning) return;
    setMode((prev) => {
      if (prev === LxmfNodeMode.BleOnly) return LxmfNodeMode.Reticulum;
      if (prev === LxmfNodeMode.Reticulum) return LxmfNodeMode.TcpClient;
      if (prev === LxmfNodeMode.TcpClient) return LxmfNodeMode.TcpServer;
      return LxmfNodeMode.BleOnly;
    });
  }, [isRunning]);

  return (
    <ScrollView
      style={styles.container}
      refreshControl={<RefreshControl refreshing={refreshing} onRefresh={handleRefresh} />}
    >
      <View style={styles.content}>
        {/* Header */}
        <Text style={styles.title}>LXMF React Native</Text>
        <Text style={styles.subtitle}>Reticulum mesh networking via Rust FFI</Text>

        {/* Identity */}
        <View style={styles.card}>
          <Text style={styles.cardTitle}>Identity</Text>
          <Row label="Native available" value={isNativeAvailable ? 'Yes' : 'No'} />
          <Row label="Node running" value={isRunning ? 'Yes' : 'No'} />
          <Row label="Identity" value={truncHex(DEV_IDENTITY_HEX)} mono />
          <Row label="LXMF Address" value={truncHex(DEV_ADDRESS_HEX)} mono />
        </View>

        {error && (
          <View style={styles.errorCard}>
            <Text style={styles.errorText}>{error}</Text>
          </View>
        )}

        {/* Interface Configuration */}
        <View style={styles.card}>
          <Text style={styles.cardTitle}>Interface Configuration</Text>
          <Text style={styles.cardHint}>Configure before starting the node</Text>

          {/* Mode selector */}
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

          {/* TCP fields — shown when TCP mode selected */}
          {mode !== LxmfNodeMode.BleOnly && (
            <View style={styles.tcpFields}>
              <Text style={styles.fieldLabel}>
                {mode === LxmfNodeMode.TcpClient ? 'Connect to host' : 'Bind address'}
              </Text>
              <TextInput
                style={styles.input}
                placeholder={mode === LxmfNodeMode.TcpClient ? '192.168.1.100' : '0.0.0.0'}
                placeholderTextColor="#484f58"
                value={tcpHost}
                onChangeText={setTcpHost}
                editable={!isRunning}
                autoCapitalize="none"
                autoCorrect={false}
                keyboardType="default"
              />
              <Text style={styles.fieldLabel}>Port</Text>
              <TextInput
                style={styles.input}
                placeholder="4242"
                placeholderTextColor="#484f58"
                value={tcpPort}
                onChangeText={setTcpPort}
                editable={!isRunning}
                keyboardType="numeric"
              />
            </View>
          )}

          {/* Announce interval */}
          <Text style={styles.fieldLabel}>Announce interval (ms)</Text>
          <TextInput
            style={styles.input}
            placeholder="5000"
            placeholderTextColor="#484f58"
            value={announceMs}
            onChangeText={setAnnounceMs}
            editable={!isRunning}
            keyboardType="numeric"
          />

          {/* Info box */}
          <View style={styles.infoBox}>
            <Text style={styles.infoText}>
              {mode === LxmfNodeMode.BleOnly &&
                'BLE Only: communicates via Bluetooth Low Energy mesh. No internet required.'}
              {mode === LxmfNodeMode.Reticulum &&
                `Reticulum TCP: connects to standard rnsd at ${tcpHost}:${tcpPort} using HDLC framing. Full interop with the Reticulum network (announces, routing, LXMF).`}
              {mode === LxmfNodeMode.TcpClient &&
                `TCP Client (FFI): connects using internal framing at ${tcpHost}:${tcpPort}. Only works with other rns-embedded-ffi nodes.`}
              {mode === LxmfNodeMode.TcpServer &&
                `TCP Server (FFI): listens on ${tcpHost}:${tcpPort} using internal framing. Only works with other rns-embedded-ffi nodes.`}
            </Text>
          </View>
        </View>

        {/* Controls */}
        <View style={styles.card}>
          <Text style={styles.cardTitle}>Controls</Text>
          <View style={styles.buttonRow}>
            <Btn
              label={isRunning ? 'Running' : 'Start'}
              onPress={handleStart}
              color={isRunning ? '#4caf50' : '#2196f3'}
              disabled={isRunning}
            />
            <Btn label="Stop" onPress={handleStop} color="#f44336" disabled={!isRunning} />
            <Btn label="Refresh" onPress={handleRefresh} color="#ff9800" />
          </View>
          <View style={styles.buttonRow}>
            <Btn label="Log: Debug" onPress={() => setLogLevel(3)} color="#9c27b0" />
            <Btn label="Log: Info" onPress={() => setLogLevel(2)} color="#9c27b0" />
            <Btn label="Log: Error" onPress={() => setLogLevel(0)} color="#9c27b0" />
          </View>
        </View>

        {/* Node Status */}
        <View style={styles.card}>
          <Text style={styles.cardTitle}>Node Status</Text>
          {nodeStatus ? (
            <>
              <Row label="Mode" value={MODE_LABELS[mode]} />
              {mode !== LxmfNodeMode.BleOnly && (
                <Row label="TCP endpoint" value={`${tcpHost}:${tcpPort}`} mono />
              )}
              <Row label="Lifecycle" value={String(nodeStatus.lifecycle)} />
              <Row label="Epoch" value={String(nodeStatus.epoch)} />
              <Row label="Pending outbound" value={String(nodeStatus.pendingOutbound)} />
              <Row label="Outbound sent" value={String(nodeStatus.outboundSent)} />
              <Row label="Inbound accepted" value={String(nodeStatus.inboundAccepted)} />
              <Row label="Announces received" value={String(nodeStatus.announcesReceived)} />
              <Row label="LXMF messages" value={String(nodeStatus.lxmfMessagesReceived)} />
            </>
          ) : (
            <Text style={styles.muted}>Pull down or tap Refresh to load status</Text>
          )}
        </View>

        {/* Send Message */}
        <View style={styles.card}>
          <Text style={styles.cardTitle}>Send Message</Text>
          <TextInput
            style={styles.input}
            placeholder="Destination hex (16 bytes / 32 chars)"
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
          <Btn label="Send" onPress={handleSend} color="#2196f3" disabled={!isRunning} />
          {sendResult && <Text style={styles.resultText}>{sendResult}</Text>}
        </View>

        {/* Beacons */}
        <View style={styles.card}>
          <Text style={styles.cardTitle}>Beacons ({discoveredBeacons.length})</Text>
          {discoveredBeacons.length > 0 ? (
            discoveredBeacons.map((b: Beacon, i: number) => (
              <View key={i} style={styles.listItem}>
                <Text style={styles.mono}>{truncHex(b.destHash)}</Text>
                <Text style={styles.muted}>
                  state: {b.state} | last: {new Date(b.lastAnnounce).toLocaleTimeString()}
                </Text>
              </View>
            ))
          ) : (
            <Text style={styles.muted}>No beacons discovered yet</Text>
          )}
        </View>

        {/* Messages */}
        <View style={styles.card}>
          <Text style={styles.cardTitle}>Messages ({messages.length})</Text>
          {messages.length > 0 ? (
            messages.map((m: any, i: number) => (
              <View key={i} style={styles.listItem}>
                <Text style={styles.mono}>{truncHex(m.source || '???')}</Text>
                <Text style={styles.muted} numberOfLines={2}>
                  {m.content ? atob(m.content) : JSON.stringify(m)}
                </Text>
              </View>
            ))
          ) : (
            <Text style={styles.muted}>No messages yet</Text>
          )}
        </View>

        {/* Event Log */}
        <View style={styles.card}>
          <Text style={styles.cardTitle}>Event Log ({eventLog.length})</Text>
          {eventLog.length > 0 ? (
            eventLog.slice(0, 20).map((line, i) => (
              <Text key={i} style={styles.logLine} numberOfLines={2}>
                {line}
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

// --- Small components ---

function Row({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <View style={styles.row}>
      <Text style={styles.rowLabel}>{label}</Text>
      <Text style={[styles.rowValue, mono && styles.mono]}>{value}</Text>
    </View>
  );
}

function Btn({
  label,
  onPress,
  color,
  disabled,
}: {
  label: string;
  onPress: () => void;
  color: string;
  disabled?: boolean;
}) {
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

// --- Styles ---

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: '#0d1117',
  },
  content: {
    paddingHorizontal: 16,
    paddingTop: 50,
  },
  title: {
    fontSize: 26,
    fontWeight: 'bold',
    color: '#e6edf3',
  },
  subtitle: {
    fontSize: 14,
    color: '#7d8590',
    marginBottom: 20,
  },
  card: {
    backgroundColor: '#161b22',
    borderRadius: 12,
    padding: 16,
    marginBottom: 12,
    borderWidth: 1,
    borderColor: '#30363d',
  },
  cardTitle: {
    fontSize: 16,
    fontWeight: '600',
    color: '#e6edf3',
    marginBottom: 10,
  },
  cardHint: {
    fontSize: 12,
    color: '#484f58',
    marginBottom: 12,
    marginTop: -6,
  },
  row: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    paddingVertical: 4,
  },
  rowLabel: {
    color: '#7d8590',
    fontSize: 14,
  },
  rowValue: {
    color: '#e6edf3',
    fontSize: 14,
  },
  mono: {
    fontFamily: 'monospace',
    fontSize: 13,
    color: '#79c0ff',
  },
  muted: {
    color: '#484f58',
    fontSize: 13,
    fontStyle: 'italic',
  },
  errorCard: {
    backgroundColor: '#3d1114',
    borderColor: '#f8514966',
    borderWidth: 1,
    borderRadius: 12,
    padding: 14,
    marginBottom: 12,
  },
  errorText: {
    color: '#f85149',
    fontSize: 14,
  },
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
  modeBadgeText: {
    color: '#fff',
    fontWeight: '600',
    fontSize: 13,
  },
  tcpFields: {
    marginBottom: 8,
  },
  fieldLabel: {
    color: '#7d8590',
    fontSize: 12,
    marginBottom: 4,
    marginTop: 4,
  },
  infoBox: {
    backgroundColor: '#0d1117',
    borderWidth: 1,
    borderColor: '#1f6feb44',
    borderRadius: 8,
    padding: 10,
    marginTop: 8,
  },
  infoText: {
    color: '#58a6ff',
    fontSize: 12,
    lineHeight: 18,
  },
  buttonRow: {
    flexDirection: 'row',
    gap: 8,
    marginBottom: 8,
  },
  btn: {
    paddingHorizontal: 14,
    paddingVertical: 8,
    borderRadius: 8,
    flex: 1,
    alignItems: 'center',
  },
  btnDisabled: {
    opacity: 0.4,
  },
  btnText: {
    color: '#fff',
    fontWeight: '600',
    fontSize: 13,
  },
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
  resultText: {
    color: '#3fb950',
    fontSize: 13,
    marginTop: 6,
  },
  listItem: {
    paddingVertical: 6,
    borderBottomWidth: 1,
    borderBottomColor: '#21262d',
  },
  logLine: {
    color: '#7d8590',
    fontSize: 11,
    fontFamily: 'monospace',
    paddingVertical: 2,
  },
});
