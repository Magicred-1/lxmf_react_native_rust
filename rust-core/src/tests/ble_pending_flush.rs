/// Regression tests for Bug 2: BLE announce receiver now caches peer identity
/// and flushes pending outbound messages (mirrors TCP-mode announce receiver).
///
/// The fix (node.rs, start_ble):
///   1. Extracts pending_sends + peer_identities arcs before spawning tasks.
///   2. Reloads persisted outbound_queue from SQLite into pending_sends on start.
///   3. BLE announce receiver: caches DestinationDesc into peer_identities and
///      drains pending_sends for the announcing peer (same as TCP mode).
///
/// What we can test without a live Reticulum peer:
///   - SQLite reload is idempotent: multiple stop/start cycles don't create
///     duplicate outbound_queue rows.
///   - Pending entries survive a stop (SQLite persists; in-memory queue clears).
///   - A second start loads entries and doesn't re-insert them into SQLite.
///
/// The announce-triggered flush itself requires a live peer announce flowing
/// through the transport, so it is not tested here (covered by manual BLE tests).

use crate::node::{LxmfNode, LxmfEvent};
use crate::store::MessageStore;
use super::NODE_LOCK;

fn unknown_dest() -> String {
    "ccddee0011223344556677889900aabb".to_string()
}

fn ble_start(db: Option<&str>) {
    LxmfNode::init(db).expect("init");
    LxmfNode::start("new", "new", 0, 60_000, 255, "[]", "test-node", false)
        .expect("start BLE");
}

fn stop() {
    let _ = LxmfNode::stop();
}

// ── SQLite outbound_queue persists across stop/start ─────────────────────────

/// After a BLE node starts, sends to an unknown dest, then stops:
/// the SQLite outbound_queue row must survive (memory clears, DB persists).
/// This is the precondition for the reload path to have anything to load.
#[test]
fn pending_entry_survives_node_stop_in_sqlite() {
    let _lock = NODE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let db = std::env::temp_dir().join("ble_flush_survive.db");
    let db_str = db.to_str().unwrap();
    let _ = std::fs::remove_file(&db);

    ble_start(Some(db_str));
    LxmfNode::send_to(&unknown_dest(), b"aGk=", None).expect("send");
    stop();

    let store = MessageStore::open(db_str).expect("open store");
    let queue = store.all_outbound_queue().expect("query queue");
    assert_eq!(queue.len(), 1, "pending entry must survive node stop");

    let _ = std::fs::remove_file(&db);
}

/// The SQLite outbound_queue row must not be duplicated when the node is
/// stopped and restarted with the same DB.  The reload copies rows into
/// the in-memory pending_sends but must NOT insert new rows into SQLite.
#[test]
fn ble_restart_with_same_store_does_not_duplicate_outbound_queue() {
    let _lock = NODE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let db = std::env::temp_dir().join("ble_flush_nodup.db");
    let db_str = db.to_str().unwrap();
    let _ = std::fs::remove_file(&db);

    // First session: enqueue one message.
    ble_start(Some(db_str));
    LxmfNode::send_to(&unknown_dest(), b"bXNn", None).expect("send");
    stop();

    let before = MessageStore::open(db_str).expect("open").all_outbound_queue().expect("query");
    assert_eq!(before.len(), 1, "setup: should have exactly 1 queued entry");

    // Second session: reload only (no new sends, no announces → no flush).
    ble_start(Some(db_str));
    stop();

    let after = MessageStore::open(db_str).expect("open").all_outbound_queue().expect("query");
    assert_eq!(after.len(), 1,
        "reload must not insert duplicate rows; expected 1 row, got {}", after.len());

    let _ = std::fs::remove_file(&db);
}

/// Three consecutive stop/start cycles must leave exactly one row in SQLite.
/// This ensures the dedup check (`!existing.contains(&seq)`) holds over
/// repeated reloads, not just the first one.
#[test]
fn multiple_restart_cycles_keep_exactly_one_sqlite_entry() {
    let _lock = NODE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let db = std::env::temp_dir().join("ble_flush_multistart.db");
    let db_str = db.to_str().unwrap();
    let _ = std::fs::remove_file(&db);

    // Enqueue once.
    ble_start(Some(db_str));
    LxmfNode::send_to(&unknown_dest(), b"dGVzdA==", None).expect("send");
    stop();

    // Three more start/stop cycles — each one reloads but must not duplicate.
    for i in 0..3 {
        ble_start(Some(db_str));
        stop();
        let count = MessageStore::open(db_str).expect("open")
            .all_outbound_queue().expect("query").len();
        assert_eq!(count, 1,
            "cycle {i}: expected 1 entry, got {count}");
    }

    let _ = std::fs::remove_file(&db);
}

/// Reload preserves the original seq number and dest address.
/// The fix must not mutate the stored rows.
#[test]
fn reload_does_not_alter_stored_seq_or_dest() {
    let _lock = NODE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let db = std::env::temp_dir().join("ble_flush_integrity.db");
    let db_str = db.to_str().unwrap();
    let _ = std::fs::remove_file(&db);

    ble_start(Some(db_str));
    let seq = LxmfNode::send_to(&unknown_dest(), b"cGF5bG9hZA==", None).expect("send");
    stop();

    let before = MessageStore::open(db_str).expect("open").all_outbound_queue().expect("query");
    let (_, stored_seq, stored_dest, _) = &before[0];
    let expected_dest = hex::decode(&unknown_dest()).expect("hex decode");
    assert_eq!(*stored_seq, seq as u64, "seq must match what send_to returned");
    assert_eq!(stored_dest, expected_dest.as_slice(), "dest must match send target");

    // Reload cycle.
    ble_start(Some(db_str));
    stop();

    let after = MessageStore::open(db_str).expect("open").all_outbound_queue().expect("query");
    assert_eq!(after.len(), 1);
    let (_, after_seq, after_dest, _) = &after[0];
    assert_eq!(after_seq, stored_seq, "seq unchanged after reload");
    assert_eq!(after_dest, stored_dest, "dest unchanged after reload");

    let _ = std::fs::remove_file(&db);
}

// ── send_to still queues after reload (no phantom delivery) ──────────────────

/// Sending to the same dest in a second session creates a second SQLite row
/// (new seq, new payload).  Confirms the node doesn't spuriously "deliver"
/// the in-memory reloaded message without a real announce.
#[test]
fn second_session_send_adds_new_entry_not_overwrite() {
    let _lock = NODE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let db = std::env::temp_dir().join("ble_flush_addentry.db");
    let db_str = db.to_str().unwrap();
    let _ = std::fs::remove_file(&db);

    // First session.
    ble_start(Some(db_str));
    let seq0 = LxmfNode::send_to(&unknown_dest(), b"bXNnMQ==", None).expect("send 1");
    stop();

    // Second session — reload + new send.
    ble_start(Some(db_str));
    let seq1 = LxmfNode::send_to(&unknown_dest(), b"bXNnMg==", None).expect("send 2");
    stop();

    // seq resets to 0 each new node session — both sends are seq=0 in their own session.
    // What matters is that both are persisted as distinct rows in SQLite.
    let queue = MessageStore::open(db_str).expect("open").all_outbound_queue().expect("query");
    assert_eq!(queue.len(), 2,
        "both sessions' messages must persist in SQLite; seq0={seq0} seq1={seq1}, rows={:?}",
        queue.iter().map(|(id, seq, _, _)| (id, seq)).collect::<Vec<_>>());
    let (id0, _, _, _) = &queue[0];
    let (id1, _, _, _) = &queue[1];
    assert_ne!(id0, id1, "SQLite row IDs must differ");

    let _ = std::fs::remove_file(&db);
}

// ── no store: in-memory only, no reload path ─────────────────────────────────

/// Without a store, send_to still returns Ok (in-memory queue only).
/// This confirms the SQLite-reload code path is safely skipped when store=None.
#[test]
fn ble_start_without_store_send_to_returns_ok() {
    let _lock = NODE_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    ble_start(None);
    let result = LxmfNode::send_to(&unknown_dest(), b"bXNn", None);
    stop();

    assert!(result.is_ok(), "send_to must succeed (queue in memory) even without a store");
}

/// Without a store, no MessageQueued event appears for a second start
/// (nothing to reload — confirms the reload branch is guarded by `if let Some(s) = store`).
#[test]
fn ble_start_without_store_no_phantom_events_on_restart() {
    let _lock = NODE_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    // First session: queue a message in memory only.
    ble_start(None);
    LxmfNode::send_to(&unknown_dest(), b"bXNn", None).expect("send");
    stop();

    // Second session: nothing to reload, no phantom events.
    ble_start(None);
    let events = LxmfNode::drain_events();
    stop();

    let queued: Vec<_> = events.iter().filter(|e| matches!(e, LxmfEvent::MessageQueued { .. })).collect();
    assert!(queued.is_empty(),
        "no MessageQueued events expected on restart without store; got {queued:?}");
}
