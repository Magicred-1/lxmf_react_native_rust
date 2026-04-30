use crate::node::{encode_lxmf_msgpack, mp_bin, mp_str, mp_array};

// ── mp_bin ────────────────────────────────────────────────────────────────────

#[test]
fn mp_bin_empty() {
    assert_eq!(mp_bin(b""), vec![0xc4, 0x00]);
}

#[test]
fn mp_bin_small() {
    assert_eq!(mp_bin(b"hi"), vec![0xc4, 0x02, b'h', b'i']);
}

#[test]
fn mp_bin_255_bytes_uses_bin8() {
    let data = vec![0xabu8; 255];
    let b = mp_bin(&data);
    assert_eq!(b[0], 0xc4);
    assert_eq!(b[1], 255);
    assert_eq!(b.len(), 257);
}

#[test]
fn mp_bin_256_bytes_uses_bin16() {
    let data = vec![0u8; 256];
    let b = mp_bin(&data);
    assert_eq!(b[0], 0xc5);
    assert_eq!((b[1] as usize) << 8 | b[2] as usize, 256);
    assert_eq!(b.len(), 259);
}

#[test]
fn mp_bin_65535_bytes_still_bin16() {
    let data = vec![0u8; 65535];
    let b = mp_bin(&data);
    assert_eq!(b[0], 0xc5);
}

#[test]
fn mp_bin_65536_bytes_uses_bin32() {
    let data = vec![0u8; 65536];
    let b = mp_bin(&data);
    assert_eq!(b[0], 0xc6);
    assert_eq!(b.len(), 5 + 65536);
}

// ── mp_str ────────────────────────────────────────────────────────────────────

#[test]
fn mp_str_empty_fixstr() {
    let s = mp_str(b"");
    assert_eq!(s, vec![0xa0]);
}

#[test]
fn mp_str_fixstr_5_chars() {
    let s = mp_str(b"hello");
    assert_eq!(s[0], 0xa5);
    assert_eq!(&s[1..], b"hello");
}

#[test]
fn mp_str_fixstr_boundary_31() {
    let s = mp_str(&[b'x'; 31]);
    assert_eq!(s[0], 0xbf); // 0xa0 | 31
    assert_eq!(s.len(), 32);
}

#[test]
fn mp_str_32_chars_uses_str8() {
    let s = mp_str(&[b'x'; 32]);
    assert_eq!(s[0], 0xd9);
    assert_eq!(s[1], 32);
    assert_eq!(s.len(), 34);
}

#[test]
fn mp_str_255_chars_uses_str8() {
    let s = mp_str(&[b'a'; 255]);
    assert_eq!(s[0], 0xd9);
    assert_eq!(s[1], 255);
}

// ── mp_array ──────────────────────────────────────────────────────────────────

#[test]
fn mp_array_empty() {
    assert_eq!(mp_array(&[]), vec![0x90]);
}

#[test]
fn mp_array_fixarray_2() {
    let entries = vec![vec![0x01u8], vec![0x02u8]];
    let a = mp_array(&entries);
    assert_eq!(a[0], 0x92);
    assert_eq!(&a[1..], [0x01, 0x02]);
}

#[test]
fn mp_array_fixarray_15_boundary() {
    let entries: Vec<Vec<u8>> = (0..15).map(|i| vec![i as u8]).collect();
    let a = mp_array(&entries);
    assert_eq!(a[0], 0x9f); // fixarray(15)
}

#[test]
fn mp_array_16_elements_uses_array16() {
    let entries: Vec<Vec<u8>> = (0..16).map(|i| vec![i as u8]).collect();
    let a = mp_array(&entries);
    assert_eq!(a[0], 0xdc);
    assert_eq!((a[1] as usize) << 8 | a[2] as usize, 16);
}

// ── encode_lxmf_msgpack ───────────────────────────────────────────────────────

#[test]
fn msgpack_starts_with_fixarray4() {
    let mp = encode_lxmf_msgpack(0.0, b"", b"hello", &[0x80]);
    assert_eq!(mp[0], 0x94);
}

#[test]
fn msgpack_second_byte_is_float64_marker() {
    let mp = encode_lxmf_msgpack(1_700_000_000.0, b"", b"", &[0x80]);
    assert_eq!(mp[1], 0xcb);
}

#[test]
fn msgpack_timestamp_roundtrips() {
    let ts = 1_700_000_000.0f64;
    let mp = encode_lxmf_msgpack(ts, b"", b"", &[0x80]);
    let bits = u64::from_be_bytes(mp[2..10].try_into().unwrap());
    assert_eq!(f64::from_bits(bits), ts);
}

#[test]
fn msgpack_empty_fields_appended_verbatim() {
    let mp = encode_lxmf_msgpack(0.0, b"", b"", &[0x80]);
    assert_eq!(*mp.last().unwrap(), 0x80);
}

#[test]
fn msgpack_custom_fields_appended_verbatim() {
    let fields = vec![0x81u8, 0x06, 0x92]; // partial fixmap — just checking passthrough
    let mp = encode_lxmf_msgpack(0.0, b"", b"", &fields);
    assert!(mp.ends_with(&fields));
}
