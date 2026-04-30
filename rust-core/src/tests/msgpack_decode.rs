use crate::node::{decode_lxmf_payload, encode_lxmf_msgpack, mp_bin, build_fields_msgpack};

/// Build fake wire payload: 96-byte zero header + msgpack.
fn wire(title: &[u8], body: &[u8], fields_mp: &[u8]) -> Vec<u8> {
    let mut w = vec![0u8; 96];
    w.extend_from_slice(&encode_lxmf_msgpack(1_700_000_000.0, title, body, fields_mp));
    w
}

// ── guard conditions ──────────────────────────────────────────────────────────

#[test]
fn too_short_returns_none() {
    assert!(decode_lxmf_payload(&[0u8; 95]).is_none());
}

#[test]
fn exactly_96_bytes_no_msgpack_returns_none() {
    assert!(decode_lxmf_payload(&[0u8; 96]).is_none());
}

#[test]
fn wrong_array_tag_returns_none() {
    let mut w = vec![0u8; 96];
    w.push(0x93); // fixarray(3) — wrong, need fixarray(4)
    assert!(decode_lxmf_payload(&w).is_none());
}

#[test]
fn non_float64_timestamp_returns_none() {
    let mut w = vec![0u8; 96];
    w.push(0x94); // fixarray(4)
    w.push(0xca); // float32 — wrong
    assert!(decode_lxmf_payload(&w).is_none());
}

// ── basic roundtrips ──────────────────────────────────────────────────────────

#[test]
fn empty_title_and_body() {
    let w = wire(b"", b"", &[0x80]);
    let dec = decode_lxmf_payload(&w).unwrap();
    assert_eq!(dec.title, b"");
    assert_eq!(dec.body, b"");
    assert!(dec.image.is_none());
    assert!(dec.files.is_empty());
}

#[test]
fn ascii_title_and_body() {
    let w = wire(b"subject", b"hello world", &[0x80]);
    let dec = decode_lxmf_payload(&w).unwrap();
    assert_eq!(dec.title, b"subject");
    assert_eq!(dec.body, b"hello world");
}

#[test]
fn body_with_all_byte_values() {
    let body: Vec<u8> = (0u8..=255u8).collect();
    let w = wire(b"", &body, &[0x80]);
    let dec = decode_lxmf_payload(&w).unwrap();
    assert_eq!(dec.body, body);
}

#[test]
fn large_body_bin16() {
    let body = vec![0xabu8; 300]; // forces bin16 in encode
    let w = wire(b"", &body, &[0x80]);
    let dec = decode_lxmf_payload(&w).unwrap();
    assert_eq!(dec.body, body);
}

#[test]
fn large_title() {
    let title = b"a".repeat(32);
    let w = wire(&title, b"body", &[0x80]);
    let dec = decode_lxmf_payload(&w).unwrap();
    assert_eq!(dec.title, title);
}

// ── field skip / unknown fields ───────────────────────────────────────────────

#[test]
fn unknown_field_before_image_skipped() {
    use crate::node::{mp_str};
    let img = b"fake_jpeg";
    let mut fields: Vec<u8> = vec![0x82]; // fixmap(2)
    // unknown field: key=0x40, bin8(4 bytes)
    fields.extend_from_slice(&[0x40, 0xc4, 0x04, 0xde, 0xad, 0xbe, 0xef]);
    // FIELD_IMAGE
    fields.push(0x06);
    fields.push(0x92);
    fields.extend_from_slice(&mp_str(b"image/png"));
    fields.extend_from_slice(&mp_bin(img));

    let w = wire(b"", b"body", &fields);
    let dec = decode_lxmf_payload(&w).unwrap();
    assert!(dec.image.is_some());
    assert_eq!(dec.image.unwrap().1, img);
}

#[test]
fn all_unknown_fields_produce_empty_image_and_files() {
    let fields: Vec<u8> = vec![0x81, 0x40, 0xc4, 0x02, 0xff, 0xee]; // fixmap(1) unknown
    let w = wire(b"", b"body", &fields);
    let dec = decode_lxmf_payload(&w).unwrap();
    assert!(dec.image.is_none());
    assert!(dec.files.is_empty());
    assert_eq!(dec.body, b"body");
}

#[test]
fn empty_map_no_fields() {
    let w = wire(b"title", b"content", &[0x80]);
    let dec = decode_lxmf_payload(&w).unwrap();
    assert!(dec.image.is_none());
    assert!(dec.files.is_empty());
}

// ── symmetry with build_fields_msgpack ───────────────────────────────────────

#[test]
fn encode_then_decode_no_media() {
    let body = b"Hello, LXMF!";
    let w = wire(b"greeting", body, &build_fields_msgpack(None));
    let dec = decode_lxmf_payload(&w).unwrap();
    assert_eq!(dec.title, b"greeting");
    assert_eq!(dec.body, body);
}

#[test]
fn binary_body_exact_roundtrip() {
    let body: Vec<u8> = (0u16..512).map(|i| (i % 256) as u8).collect();
    let w = wire(b"", &body, &[0x80]);
    assert_eq!(decode_lxmf_payload(&w).unwrap().body, body);
}
