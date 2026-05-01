import { useCallback, useState } from 'react';
import {
  FlatList,
  Modal,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
} from 'react-native';
import { useRouter } from 'expo-router';
import { useLxmfContext, type Contact } from '@/context/LxmfContext';

// ── Helpers ───────────────────────────────────────────────────────────────────

function shortHex(v: string): string {
  if (!v || v.length <= 12) return v || '—';
  return `${v.slice(0, 6)}…${v.slice(-6)}`;
}

function relTime(unix: number): string {
  const diff = Math.floor(Date.now() / 1000) - unix;
  if (diff < 60) return 'now';
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}

// ── Contact row ───────────────────────────────────────────────────────────────

function ContactRow({ contact, onPress }: Readonly<{ contact: Contact; onPress: () => void }>) {
  const label = contact.name || shortHex(contact.address);
  return (
    <Pressable
      style={({ pressed }) => [S.row, pressed && S.rowPressed]}
      onPress={onPress}>
      <View style={S.avatar}>
        <Text style={S.avatarText}>{label.slice(0, 2).toUpperCase()}</Text>
      </View>
      <View style={S.rowBody}>
        <View style={S.rowTop}>
          <Text style={S.rowName} numberOfLines={1}>{label}</Text>
          <Text style={S.rowTime}>{relTime(contact.lastSeen)}</Text>
        </View>
        <View style={S.rowBottom}>
          <Text style={S.rowPreview} numberOfLines={1}>
            {contact.lastMessage || 'No messages yet'}
          </Text>
          {contact.unread > 0 && (
            <View style={S.badge}>
              <Text style={S.badgeText}>{contact.unread > 99 ? '99+' : contact.unread}</Text>
            </View>
          )}
        </View>
      </View>
    </Pressable>
  );
}

// ── Main screen ───────────────────────────────────────────────────────────────

export default function ConversationsScreen() {
  const { contacts, upsertContact, isRunning } = useLxmfContext();
  const router = useRouter();
  const [showNew, setShowNew] = useState(false);
  const [newAddr, setNewAddr] = useState('');
  const [addrError, setAddrError] = useState('');

  const openThread = useCallback((address: string) => {
    router.push(`/conversation/${address}`);
  }, [router]);

  const addContact = useCallback(() => {
    const addr = newAddr.trim().toLowerCase();
    if (!/^[0-9a-f]{32}$/.test(addr)) {
      setAddrError('Must be 32 hex characters.');
      return;
    }
    upsertContact(addr);
    setShowNew(false);
    setNewAddr('');
    setAddrError('');
    openThread(addr);
  }, [newAddr, upsertContact, openThread]);

  const renderItem = useCallback(({ item }: { item: Contact }) => (
    <ContactRow contact={item} onPress={() => openThread(item.address)} />
  ), [openThread]);

  const keyExtractor = useCallback((item: Contact) => item.address, []);

  return (
    <View style={S.root}>
      <View style={S.header}>
        <Text style={S.headerTitle}>Messages</Text>
        {!isRunning && (
          <Text style={S.headerHint}>Start node in Network tab to receive messages.</Text>
        )}
      </View>

      {contacts.length === 0 ? (
        <View style={S.empty}>
          <Text style={S.emptyTitle}>No contacts yet</Text>
          <Text style={S.emptyBody}>
            Peer announces appear here automatically.{'\n'}
            Tap + to message a known address.
          </Text>
        </View>
      ) : (
        <FlatList
          data={contacts}
          keyExtractor={keyExtractor}
          renderItem={renderItem}
          contentContainerStyle={S.list}
          ItemSeparatorComponent={Separator}
        />
      )}

      {/* FAB */}
      <Pressable style={({ pressed }) => [S.fab, pressed && S.fabPressed]} onPress={() => setShowNew(true)}>
        <Text style={S.fabText}>+</Text>
      </Pressable>

      {/* New conversation modal */}
      <Modal visible={showNew} transparent animationType="fade" onRequestClose={() => setShowNew(false)}>
        <Pressable style={S.overlay} onPress={() => setShowNew(false)}>
          <Pressable style={S.modal}>
            <Text style={S.modalTitle}>New Conversation</Text>
            <Text style={S.modalHint}>Enter 32-character LXMF address (hex)</Text>
            <TextInput
              style={S.modalInput}
              placeholder="aabbccdd…"
              placeholderTextColor="#4a6070"
              value={newAddr}
              onChangeText={t => { setNewAddr(t); setAddrError(''); }}
              autoCapitalize="none"
              autoCorrect={false}
              autoFocus
            />
            {addrError ? <Text style={S.modalError}>{addrError}</Text> : null}
            <View style={S.modalBtns}>
              <Pressable style={[S.modalBtn, S.modalBtnCancel]} onPress={() => { setShowNew(false); setNewAddr(''); setAddrError(''); }}>
                <Text style={S.modalBtnText}>Cancel</Text>
              </Pressable>
              <Pressable style={[S.modalBtn, S.modalBtnOk]} onPress={addContact}>
                <Text style={S.modalBtnText}>Open</Text>
              </Pressable>
            </View>
          </Pressable>
        </Pressable>
      </Modal>
    </View>
  );
}

function Separator() {
  return <View style={S.separator} />;
}

// ── Styles ────────────────────────────────────────────────────────────────────

const C = {
  bg: '#0c1218',
  surface: '#131d26',
  border: '#1e3040',
  accent: '#1a7fc1',
  accentBright: '#4fb3e8',
  text: '#d8ecf8',
  textDim: '#7a9db5',
  warn: '#f0a500',
};

const S = StyleSheet.create({
  root: { flex: 1, backgroundColor: C.bg },

  header: {
    paddingHorizontal: 16,
    paddingTop: 56,
    paddingBottom: 12,
    backgroundColor: C.surface,
    borderBottomWidth: 1,
    borderBottomColor: C.border,
  },
  headerTitle: { color: C.text, fontSize: 28, fontWeight: '700' },
  headerHint: { color: C.warn, fontSize: 12, marginTop: 4 },

  list: { paddingBottom: 80 },
  separator: { height: 1, backgroundColor: C.border, marginLeft: 72 },

  row: { flexDirection: 'row', alignItems: 'center', paddingHorizontal: 16, paddingVertical: 12, backgroundColor: C.surface },
  rowPressed: { backgroundColor: '#17232e' },

  avatar: {
    width: 44,
    height: 44,
    borderRadius: 22,
    backgroundColor: '#0d3550',
    borderWidth: 1,
    borderColor: C.accentBright,
    alignItems: 'center',
    justifyContent: 'center',
    marginRight: 12,
  },
  avatarText: { color: C.accentBright, fontSize: 14, fontWeight: '700' },

  rowBody: { flex: 1 },
  rowTop: { flexDirection: 'row', justifyContent: 'space-between', alignItems: 'baseline', marginBottom: 3 },
  rowName: { color: C.text, fontSize: 15, fontWeight: '600', flex: 1, marginRight: 8 },
  rowTime: { color: C.textDim, fontSize: 12 },

  rowBottom: { flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between' },
  rowPreview: { color: C.textDim, fontSize: 13, flex: 1, marginRight: 8 },

  badge: {
    backgroundColor: C.accent,
    borderRadius: 10,
    minWidth: 20,
    height: 20,
    alignItems: 'center',
    justifyContent: 'center',
    paddingHorizontal: 5,
  },
  badgeText: { color: '#fff', fontSize: 11, fontWeight: '700' },

  empty: { flex: 1, alignItems: 'center', justifyContent: 'center', paddingHorizontal: 40 },
  emptyTitle: { color: C.text, fontSize: 20, fontWeight: '600', marginBottom: 10 },
  emptyBody: { color: C.textDim, fontSize: 14, textAlign: 'center', lineHeight: 22 },

  fab: {
    position: 'absolute',
    right: 20,
    bottom: 24,
    width: 54,
    height: 54,
    borderRadius: 27,
    backgroundColor: C.accent,
    alignItems: 'center',
    justifyContent: 'center',
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 3 },
    shadowOpacity: 0.4,
    shadowRadius: 6,
    elevation: 6,
  },
  fabPressed: { opacity: 0.8 },
  fabText: { color: '#fff', fontSize: 28, lineHeight: 32, fontWeight: '300' },

  overlay: { flex: 1, backgroundColor: 'rgba(0,0,0,0.7)', justifyContent: 'center', alignItems: 'center', padding: 24 },
  modal: { width: '100%', backgroundColor: C.surface, borderRadius: 16, borderWidth: 1, borderColor: C.border, padding: 20, gap: 12 },
  modalTitle: { color: C.text, fontSize: 18, fontWeight: '700' },
  modalHint: { color: C.textDim, fontSize: 13 },
  modalInput: {
    borderWidth: 1, borderColor: '#2a4050', backgroundColor: '#0b1820', color: C.text,
    borderRadius: 10, paddingHorizontal: 12, paddingVertical: 10,
    fontFamily: 'monospace', fontSize: 13,
  },
  modalError: { color: '#ff7070', fontSize: 12 },
  modalBtns: { flexDirection: 'row', gap: 10, marginTop: 4 },
  modalBtn: { flex: 1, borderRadius: 10, paddingVertical: 11, alignItems: 'center' },
  modalBtnCancel: { backgroundColor: '#1a2e40' },
  modalBtnOk: { backgroundColor: C.accent },
  modalBtnText: { color: '#e8f6ff', fontSize: 14, fontWeight: '600' },
});
