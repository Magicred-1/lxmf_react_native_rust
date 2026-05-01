/// Integration tests for the node-level send queue and path-request fix.
///
/// What we're verifying:
/// 1. send_to an unknown destination → MessageQueued event emitted
/// 2. MessageQueued carries the correct dest_hex
/// 3. Multiple sends → multiple MessageQueued events, one per send
///
/// The path request spawn (fired on DroppedMissingDestinationIdentity) is also
/// exercised — it must not panic. In BLE-only mode with no peers the packet
/// finds no route, but the code path runs without error.
///
/// Node tests share the global LxmfNode singleton so they must be serialised.

use crate::node::{LxmfNode, LxmfEvent};
use std::sync::Mutex;

/// Serialize all node lifecycle tests; parallel tests racing on the global NODE
/// singleton would corrupt each other's state.
static NODE_LOCK: Mutex<()> = Mutex::new(());

/// Unknown 32-hex-char destination that has no identity in the transport table.
fn unknown_dest_hex() -> String {
    "aabbccddeeff00112233445566778899".to_string()
}

fn unknown_dest_hex_2() -> String {
    "112233445566778899aabbccddeeff00".to_string()
}

/// Drain events and stop the node. Convenience for test teardown.
fn stop_and_drain() -> Vec<LxmfEvent> {
    let events = LxmfNode::drain_events();
    let _ = LxmfNode::stop();
    events
}

// ── send_to unknown destination queues message ────────────────────────────────

#[test]
fn send_to_unknown_emits_message_queued_event() {
    let _lock = NODE_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    LxmfNode::init(None).expect("init");
    // BLE-only mode — transport starts without external peers; fast (200ms delay).
    LxmfNode::start("new", "new", 0, 60_000, 255, "[]", "test-node", false)
        .expect("start");

    let dest = unknown_dest_hex();
    let body_b64 = "aGVsbG8="; // base64("hello")
    LxmfNode::send_to(&dest, body_b64.as_bytes(), None)
        .expect("send_to should return Ok (queued)");

    let events = stop_and_drain();
    let queued: Vec<&LxmfEvent> = events.iter().filter(|e| {
        matches!(e, LxmfEvent::MessageQueued { .. })
    }).collect();

    assert!(!queued.is_empty(), "expected at least one MessageQueued event; got: {:?}", events);
}

#[test]
fn send_to_unknown_message_queued_has_correct_dest_hex() {
    let _lock = NODE_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    LxmfNode::init(None).expect("init");
    LxmfNode::start("new", "new", 0, 60_000, 255, "[]", "test-node", false)
        .expect("start");

    let dest = unknown_dest_hex();
    LxmfNode::send_to(&dest, b"dGVzdA==", None)
        .expect("send_to should queue");

    let events = stop_and_drain();
    let found = events.iter().any(|e| {
        matches!(e, LxmfEvent::MessageQueued { dest_hex, .. } if dest_hex == &dest)
    });
    assert!(found, "MessageQueued event with dest={dest} not found in events: {:?}", events);
}

#[test]
fn send_to_multiple_unknown_dests_queues_each() {
    let _lock = NODE_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    LxmfNode::init(None).expect("init");
    LxmfNode::start("new", "new", 0, 60_000, 255, "[]", "test-node", false)
        .expect("start");

    let dest1 = unknown_dest_hex();
    let dest2 = unknown_dest_hex_2();

    LxmfNode::send_to(&dest1, b"bXNnMQ==", None).expect("send 1");
    LxmfNode::send_to(&dest2, b"bXNnMg==", None).expect("send 2");

    let events = stop_and_drain();
    let queued_dests: Vec<String> = events.iter().filter_map(|e| {
        if let LxmfEvent::MessageQueued { dest_hex, .. } = e {
            Some(dest_hex.clone())
        } else {
            None
        }
    }).collect();

    assert!(
        queued_dests.contains(&dest1),
        "MessageQueued for dest1 missing; queued_dests={queued_dests:?}"
    );
    assert!(
        queued_dests.contains(&dest2),
        "MessageQueued for dest2 missing; queued_dests={queued_dests:?}"
    );
}

#[test]
fn send_to_unknown_seq_is_monotone() {
    let _lock = NODE_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    LxmfNode::init(None).expect("init");
    LxmfNode::start("new", "new", 0, 60_000, 255, "[]", "test-node", false)
        .expect("start");

    let dest = unknown_dest_hex();
    let seq0 = LxmfNode::send_to(&dest, b"bXNnMQ==", None).expect("send 0");
    let seq1 = LxmfNode::send_to(&dest, b"bXNnMg==", None).expect("send 1");
    let seq2 = LxmfNode::send_to(&dest, b"bXNnMw==", None).expect("send 2");

    let _ = stop_and_drain();
    assert!(seq1 > seq0, "seq should increment: seq0={seq0} seq1={seq1}");
    assert!(seq2 > seq1, "seq should increment: seq1={seq1} seq2={seq2}");
}

// ── outbound queue persisted when store is enabled ────────────────────────────

#[test]
fn send_to_unknown_with_store_enqueues_in_sqlite() {
    let _lock = NODE_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    // Use a temp file for the DB so we can verify the outbound queue was written.
    let db_path = std::env::temp_dir().join("lxmf_test_queue.db");
    let db_str = db_path.to_str().expect("valid path");

    // Clean up any leftover from a previous run.
    let _ = std::fs::remove_file(&db_path);

    LxmfNode::init(Some(db_str)).expect("init with store");
    LxmfNode::start("new", "new", 0, 60_000, 255, "[]", "test-node", false)
        .expect("start");

    let dest = unknown_dest_hex();
    LxmfNode::send_to(&dest, b"cGF5bG9hZA==", None).expect("send_to");

    let _ = stop_and_drain();

    // Verify the outbound queue row was written to SQLite.
    let store = crate::store::MessageStore::open(db_str).expect("open store");
    let queue = store.all_outbound_queue().expect("list queue");
    assert!(
        !queue.is_empty(),
        "expected queued row in outbound_queue after send_to unknown dest"
    );

    let _ = std::fs::remove_file(&db_path);
}
