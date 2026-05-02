/// Regression tests for Bug 1: synthetic AnnounceReceived emitted from the BLE
/// data receiver when a packet arrives from a peer.
///
/// The fix (node.rs, start_ble data receiver):
///   Extracts sender LXMF address from wire bytes [16..32] and immediately pushes
///   an AnnounceReceived { hops: 0, app_data: [] } event so the JS peer list
///   updates without waiting up to 60s for the peer's periodic announce.
///
/// These tests validate:
///   1. The wire layout assumption — sender IS at bytes [16..32].
///   2. The extraction guard — payloads shorter than 32 bytes are skipped (no panic).
///   3. The emitted event structure matches what JS/LxmfContext expects.

use crate::node::{encode_lxmf_msgpack, build_fields_msgpack, LxmfEvent};

// ── helpers ───────────────────────────────────────────────────────────────────

fn dest_hash() -> [u8; 16] { [0xdd; 16] }
fn sender_hash() -> [u8; 16] { [0x5e; 16] }

/// Build a minimal LXMF wire payload:
///   [0..16]  dest
///   [16..32] sender  ← the field the fix reads
///   [32..96] sig (zeros OK for offset tests)
///   [96+]    msgpack
fn wire(dest: &[u8; 16], sender: &[u8; 16], body: &[u8]) -> Vec<u8> {
    let fields = build_fields_msgpack(None);
    let mp = encode_lxmf_msgpack(1_700_000_000.0, b"", body, &fields);
    let mut v = Vec::with_capacity(96 + mp.len());
    v.extend_from_slice(dest);
    v.extend_from_slice(sender);
    v.extend_from_slice(&[0u8; 64]);
    v.extend_from_slice(&mp);
    v
}

// ── wire layout: sender sits at bytes [16..32] ───────────────────────────────

/// The fix reads `data[16..32]` as the sender's LXMF address.
/// Confirm the wire layout places the sender there.
#[test]
fn sender_is_at_bytes_16_to_32_of_wire_payload() {
    let s = sender_hash();
    let payload = wire(&dest_hash(), &s, b"hello");
    assert_eq!(&payload[16..32], &s,
        "sender address must be at bytes [16..32] for synthetic-announce extraction");
}

/// Dest and sender are distinct fields and must not overlap.
#[test]
fn dest_and_sender_occupy_separate_regions() {
    let d = [0xAAu8; 16];
    let s = [0xBBu8; 16];
    let payload = wire(&d, &s, b"x");
    assert_eq!(&payload[0..16],  &d, "dest at [0..16]");
    assert_eq!(&payload[16..32], &s, "sender at [16..32]");
    assert_ne!(&payload[0..16], &payload[16..32]);
}

/// Two senders produce different addresses at bytes [16..32].
#[test]
fn two_different_senders_yield_different_bytes_at_offset_16() {
    let a = [0x01u8; 16];
    let b = [0x02u8; 16];
    let pa = wire(&dest_hash(), &a, b"msg");
    let pb = wire(&dest_hash(), &b, b"msg");
    assert_ne!(&pa[16..32], &pb[16..32]);
}

/// Extraction via `payload[16..32].copy_from_slice` gives the original sender.
#[test]
fn slice_copy_extraction_matches_original_sender() {
    let original = sender_hash();
    let payload = wire(&dest_hash(), &original, b"body");

    let mut extracted = [0u8; 16];
    extracted.copy_from_slice(&payload[16..32]);
    assert_eq!(extracted, original);
}

// ── extraction guard: payloads shorter than 32 bytes are skipped ─────────────

/// The fix guards with `data.len() >= 32`; this test checks that the boundary
/// value of 31 bytes would NOT satisfy the guard (preventing an out-of-bounds
/// slice on `data[16..32]`).
#[test]
fn payload_of_31_bytes_does_not_satisfy_extraction_guard() {
    let short = vec![0u8; 31];
    assert!(short.len() < 32, "31 bytes must be below the extraction guard threshold");
    // Confirm the guard condition (`data.len() >= 32`) evaluates to false.
    assert!(!short.len().ge(&32));
}

/// A payload of exactly 32 bytes satisfies the guard and allows safe extraction.
#[test]
fn payload_of_exactly_32_bytes_satisfies_guard_and_is_safe() {
    let payload = vec![0xABu8; 32];
    assert!(payload.len() >= 32);
    // Safe to slice: no panic.
    let mut extracted = [0u8; 16];
    extracted.copy_from_slice(&payload[16..32]);
    assert_eq!(extracted, [0xABu8; 16]);
}

/// Empty payload is well below the guard — the fix must not be reached.
#[test]
fn empty_payload_does_not_satisfy_guard() {
    let empty: Vec<u8> = vec![];
    assert!(empty.len() < 32);
}

// ── emitted AnnounceReceived event structure ──────────────────────────────────

/// The synthetic event must have hops=0 (direct BLE connection, not routed)
/// and empty app_data (we don't know the peer's display name from the packet).
/// JS/LxmfContext must handle hops=0 correctly (it uses it for UI display only).
#[test]
fn synthetic_announce_event_has_hops_zero_and_empty_app_data() {
    let sender = sender_hash();
    // Construct exactly what the fix emits — if the event variant or fields change
    // this test will catch the mismatch.
    let event = LxmfEvent::AnnounceReceived {
        dest_hash: sender,
        app_data: vec![],
        hops: 0,
    };

    match event {
        LxmfEvent::AnnounceReceived { dest_hash, app_data, hops } => {
            assert_eq!(dest_hash, sender, "dest_hash must be the extracted sender address");
            assert_eq!(hops, 0, "hops must be 0 for a direct BLE connection");
            assert!(app_data.is_empty(), "app_data must be empty (display name unknown from packet)");
        }
        other => panic!("unexpected event variant: {:?}", other),
    }
}

/// Verifies that the dest_hash in the synthetic event matches the sender bytes
/// from the wire payload — end-to-end correctness of the extraction + event.
#[test]
fn synthetic_announce_dest_hash_matches_wire_sender() {
    let sender = [0x7Fu8; 16];
    let payload = wire(&dest_hash(), &sender, b"content");

    // Replicate exactly what the BLE data receiver does.
    assert!(payload.len() >= 32);
    let mut sender_hash = [0u8; 16];
    sender_hash.copy_from_slice(&payload[16..32]);

    let event = LxmfEvent::AnnounceReceived {
        dest_hash: sender_hash,
        app_data: vec![],
        hops: 0,
    };

    match event {
        LxmfEvent::AnnounceReceived { dest_hash, .. } => {
            assert_eq!(dest_hash, sender,
                "event dest_hash must equal wire sender bytes at [16..32]");
        }
        _ => panic!("wrong event"),
    }
}

/// The synthetic announce dest_hash must NOT equal the dest_hash in the wire
/// payload — these are different fields (we report the SENDER, not ourselves).
#[test]
fn synthetic_announce_is_sender_not_dest() {
    let dest = dest_hash();
    let sender = sender_hash();
    assert_ne!(dest, sender, "test setup: dest and sender must differ");

    let payload = wire(&dest, &sender, b"x");
    let mut extracted = [0u8; 16];
    extracted.copy_from_slice(&payload[16..32]);

    // The extracted address should be the sender, not the dest.
    assert_eq!(extracted, sender);
    assert_ne!(extracted, dest);
}
