import { useCallback, useState } from 'react';
import {
  Alert,
  Modal,
  Pressable,
  ScrollView,
  Share,
  StyleSheet,
  Text,
  TextInput,
  View,
} from 'react-native';
import { useLxmfContext } from '@/context/LxmfContext';

function shortHex(v: string): string {
  if (!v || v.length <= 12) return v || '—';
  return `${v.slice(0, 6)}…${v.slice(-6)}`;
}

function Row({ label, value, onAction, actionLabel }: Readonly<{
  label: string; value: string; onAction?: () => void; actionLabel?: string;
}>) {
  return (
    <View style={S.row}>
      <View style={S.rowLeft}>
        <Text style={S.rowLabel}>{label}</Text>
        <Text selectable style={S.rowValue}>{value}</Text>
      </View>
      {onAction && actionLabel && (
        <Pressable style={({ pressed }) => [S.actionBtn, pressed && { opacity: 0.7 }]} onPress={onAction}>
          <Text style={S.actionBtnText}>{actionLabel}</Text>
        </Pressable>
      )}
    </View>
  );
}

const LOG_LEVELS = ['0 – Off', '1 – Error', '2 – Warn', '3 – Info', '4 – Debug', '5 – Trace'];

export default function SettingsScreen() {
  const {
    identity, status, isRunning, displayName, setDisplayName, clearIdentity, setLogLevel,
  } = useLxmfContext();

  const [nameInput, setNameInput] = useState(displayName);
  const [logLevel, setLogLevelState] = useState(3);
  const [confirmReset, setConfirmReset] = useState(false);
  const [nameSaved, setNameSaved] = useState(false);

  const saveDisplayName = useCallback(() => {
    setDisplayName(nameInput.trim() || 'lxmf-mobile');
    setNameSaved(true);
    setTimeout(() => setNameSaved(false), 2000);
  }, [nameInput, setDisplayName]);

  const changeLogLevel = useCallback((delta: number) => {
    const next = Math.max(0, Math.min(5, logLevel + delta));
    setLogLevelState(next);
    setLogLevel(next);
  }, [logLevel, setLogLevel]);

  const copyAddress = useCallback(() => {
    const addr = status?.addressHex ?? identity?.address_hex;
    if (addr) Share.share({ message: addr }).catch(() => {});
  }, [status?.addressHex, identity?.address_hex]);

  const doReset = useCallback(async () => {
    setConfirmReset(false);
    await clearIdentity();
  }, [clearIdentity]);

  const displayAddr = status?.addressHex ?? identity?.address_hex ?? null;
  const createdAt = identity?.created_at
    ? new Date(identity.created_at).toLocaleDateString()
    : '—';

  return (
    <ScrollView style={S.root} contentContainerStyle={S.scroll}>
      <View style={S.header}>
        <Text style={S.headerTitle}>Settings</Text>
      </View>

      {/* Display name */}
      <View style={S.section}>
        <Text style={S.sectionTitle}>Display Name</Text>
        <Text style={S.hint}>Shown to peers in LXMF announces.</Text>
        <View style={S.inputRow}>
          <TextInput
            style={[S.input, S.inputFlex]}
            value={nameInput}
            onChangeText={setNameInput}
            placeholder="lxmf-mobile"
            placeholderTextColor="#4a6070"
            autoCapitalize="none"
            autoCorrect={false}
          />
          <Pressable style={({ pressed }) => [S.saveBtn, pressed && { opacity: 0.75 }]} onPress={saveDisplayName}>
            <Text style={S.saveBtnText}>{nameSaved ? '✓' : 'Save'}</Text>
          </Pressable>
        </View>
      </View>

      {/* Identity */}
      <View style={S.section}>
        <Text style={S.sectionTitle}>My Identity</Text>
        {displayAddr ? (
          <>
            <Row
              label="Address"
              value={shortHex(displayAddr)}
              onAction={copyAddress}
              actionLabel="⎘ Copy"
            />
            <Row label="Full address" value={displayAddr} />
            <Row label="Created" value={createdAt} />
          </>
        ) : (
          <Text style={S.hint}>No identity yet — start the node in the Network tab.</Text>
        )}
      </View>

      {/* Log level */}
      <View style={S.section}>
        <Text style={S.sectionTitle}>Log Level</Text>
        <View style={S.logRow}>
          <Pressable style={({ pressed }) => [S.stepBtn, pressed && { opacity: 0.7 }]} onPress={() => changeLogLevel(-1)}>
            <Text style={S.stepBtnText}>−</Text>
          </Pressable>
          <Text style={S.logLabel}>{LOG_LEVELS[logLevel] ?? String(logLevel)}</Text>
          <Pressable style={({ pressed }) => [S.stepBtn, pressed && { opacity: 0.7 }]} onPress={() => changeLogLevel(1)}>
            <Text style={S.stepBtnText}>+</Text>
          </Pressable>
        </View>
      </View>

      {/* Danger zone */}
      <View style={[S.section, S.dangerSection]}>
        <Text style={[S.sectionTitle, S.dangerTitle]}>Danger Zone</Text>
        <Text style={S.hint}>
          Resetting your identity permanently removes your private key. You will lose access to your LXMF address and any pending messages.
        </Text>
        <Pressable
          style={({ pressed }) => [S.dangerBtn, isRunning && S.btnDisabled, pressed && { opacity: 0.8 }]}
          onPress={() => setConfirmReset(true)}
          disabled={isRunning}>
          <Text style={S.dangerBtnText}>{isRunning ? 'Stop node before resetting' : 'Reset Identity'}</Text>
        </Pressable>
      </View>

      {/* Confirm reset modal */}
      <Modal visible={confirmReset} transparent animationType="fade" onRequestClose={() => setConfirmReset(false)}>
        <Pressable style={S.overlay} onPress={() => setConfirmReset(false)}>
          <Pressable style={S.modal}>
            <Text style={S.modalTitle}>Reset Identity?</Text>
            <Text style={S.modalBody}>
              This permanently deletes your identity and address.{'\n\n'}
              You will lose access to all pending messages. This cannot be undone.
            </Text>
            <View style={S.modalBtns}>
              <Pressable style={[S.modalBtn, S.modalBtnCancel]} onPress={() => setConfirmReset(false)}>
                <Text style={S.modalBtnText}>Cancel</Text>
              </Pressable>
              <Pressable style={[S.modalBtn, S.modalBtnDanger]} onPress={doReset}>
                <Text style={S.modalBtnText}>Delete</Text>
              </Pressable>
            </View>
          </Pressable>
        </Pressable>
      </Modal>
    </ScrollView>
  );
}

// ── Styles ────────────────────────────────────────────────────────────────────

const S = StyleSheet.create({
  root: { flex: 1, backgroundColor: '#0c1218' },
  scroll: { paddingBottom: 60, gap: 12 },

  header: {
    paddingHorizontal: 16, paddingTop: 56, paddingBottom: 14,
    backgroundColor: '#131d26', borderBottomWidth: 1, borderBottomColor: '#1e3040',
  },
  headerTitle: { color: '#d8ecf8', fontSize: 28, fontWeight: '700' },

  section: {
    backgroundColor: '#131d26', borderRadius: 14, borderWidth: 1,
    borderColor: '#1e3040', padding: 16, gap: 10, marginHorizontal: 14,
  },
  sectionTitle: { color: '#d8ecf8', fontSize: 15, fontWeight: '700' },
  hint: { color: '#7a9db5', fontSize: 13, lineHeight: 19 },

  row: { flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', gap: 10 },
  rowLeft: { flex: 1, gap: 2 },
  rowLabel: { color: '#7a9db5', fontSize: 12 },
  rowValue: { color: '#d8ecf8', fontSize: 13, fontFamily: 'monospace' },

  actionBtn: { paddingHorizontal: 10, paddingVertical: 5, borderRadius: 8, backgroundColor: '#0d3550', borderWidth: 1, borderColor: '#1e3040' },
  actionBtnText: { color: '#4fb3e8', fontSize: 12, fontWeight: '600' },

  inputRow: { flexDirection: 'row', gap: 8, alignItems: 'center' },
  inputFlex: { flex: 1 },
  input: {
    borderWidth: 1, borderColor: '#2a4050', backgroundColor: '#0b1820', color: '#d8ecf8',
    borderRadius: 10, paddingHorizontal: 12, paddingVertical: 10, fontSize: 14,
  },
  saveBtn: { paddingHorizontal: 14, paddingVertical: 10, borderRadius: 10, backgroundColor: '#1a7fc1' },
  saveBtnText: { color: '#e8f6ff', fontSize: 14, fontWeight: '600' },

  logRow: { flexDirection: 'row', alignItems: 'center', gap: 14 },
  stepBtn: { width: 36, height: 36, borderRadius: 18, backgroundColor: '#0d3550', borderWidth: 1, borderColor: '#1e3040', alignItems: 'center', justifyContent: 'center' },
  stepBtnText: { color: '#4fb3e8', fontSize: 20, lineHeight: 24, fontWeight: '600' },
  logLabel: { flex: 1, color: '#d8ecf8', fontSize: 14, fontFamily: 'monospace' },

  dangerSection: { borderColor: '#4a1515' },
  dangerTitle: { color: '#ff7070' },
  dangerBtn: { backgroundColor: '#7a1515', borderRadius: 10, paddingVertical: 12, alignItems: 'center' },
  dangerBtnText: { color: '#ffcccc', fontSize: 14, fontWeight: '600' },
  btnDisabled: { opacity: 0.4 },

  overlay: { flex: 1, backgroundColor: 'rgba(0,0,0,0.75)', justifyContent: 'center', alignItems: 'center', padding: 24 },
  modal: { width: '100%', backgroundColor: '#131d26', borderRadius: 16, borderWidth: 1, borderColor: '#4a1515', padding: 20, gap: 14 },
  modalTitle: { color: '#ff7070', fontSize: 18, fontWeight: '700' },
  modalBody: { color: '#d8ecf8', fontSize: 14, lineHeight: 22 },
  modalBtns: { flexDirection: 'row', gap: 10 },
  modalBtn: { flex: 1, borderRadius: 10, paddingVertical: 11, alignItems: 'center' },
  modalBtnCancel: { backgroundColor: '#1a2e40' },
  modalBtnDanger: { backgroundColor: '#7a1515' },
  modalBtnText: { color: '#e8f6ff', fontSize: 14, fontWeight: '600' },
});
