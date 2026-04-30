use base64::Engine as _;
use crate::node::{build_fields_msgpack, decode_lxmf_payload, encode_lxmf_msgpack};

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

fn wire_with_media(media_json: &str) -> Vec<u8> {
    let mut w = vec![0u8; 96];
    w.extend_from_slice(&encode_lxmf_msgpack(
        0.0, b"", b"body",
        &build_fields_msgpack(Some(media_json)),
    ));
    w
}

// ── build_fields_msgpack: empty / null inputs ─────────────────────────────────

#[test]
fn none_gives_empty_map() {
    assert_eq!(build_fields_msgpack(None), vec![0x80]);
}

#[test]
fn empty_string_gives_empty_map() {
    assert_eq!(build_fields_msgpack(Some("")), vec![0x80]);
}

#[test]
fn null_string_gives_empty_map() {
    assert_eq!(build_fields_msgpack(Some("null")), vec![0x80]);
}

#[test]
fn empty_object_gives_empty_map() {
    assert_eq!(build_fields_msgpack(Some("{}")), vec![0x80]);
}

#[test]
fn invalid_json_gives_empty_map() {
    assert_eq!(build_fields_msgpack(Some("not json")), vec![0x80]);
}

// ── FIELD_IMAGE (0x06) encoding ───────────────────────────────────────────────

#[test]
fn image_produces_fixmap1_with_key_0x06() {
    let json = format!(r#"{{"image":{{"mimeType":"image/jpeg","data":"{}"}}}}"#, B64.encode(b"x"));
    let mp = build_fields_msgpack(Some(&json));
    assert_eq!(mp[0], 0x81, "fixmap(1)");
    assert_eq!(mp[1], 0x06, "FIELD_IMAGE");
    assert_eq!(mp[2], 0x92, "fixarray(2) value");
}

#[test]
fn image_bad_base64_skipped_empty_map() {
    let mp = build_fields_msgpack(Some(r#"{"image":{"mimeType":"image/jpeg","data":"!!!bad"}}"#));
    assert_eq!(mp, vec![0x80]);
}

#[test]
fn image_missing_data_field_skipped() {
    let mp = build_fields_msgpack(Some(r#"{"image":{"mimeType":"image/jpeg"}}"#));
    assert_eq!(mp, vec![0x80]);
}

// ── FIELD_FILE_ATTACHMENTS (0x05) encoding ────────────────────────────────────

#[test]
fn single_file_produces_key_0x05_fixarray1() {
    let json = format!(r#"{{"files":[{{"name":"f.txt","data":"{}"}}]}}"#, B64.encode(b"data"));
    let mp = build_fields_msgpack(Some(&json));
    assert_eq!(mp[0], 0x81, "fixmap(1)");
    assert_eq!(mp[1], 0x05, "FIELD_FILE_ATTACHMENTS");
    assert_eq!(mp[2], 0x91, "fixarray(1)");
}

#[test]
fn three_files_produces_array3() {
    let json = format!(
        r#"{{"files":[{{"name":"a","data":"{}"}},{{"name":"b","data":"{}"}},{{"name":"c","data":"{}"}}]}}"#,
        B64.encode(b"1"), B64.encode(b"2"), B64.encode(b"3"),
    );
    let mp = build_fields_msgpack(Some(&json));
    assert_eq!(mp[1], 0x05);
    assert_eq!(mp[2], 0x93, "fixarray(3)");
}

#[test]
fn empty_files_array_skipped_empty_map() {
    assert_eq!(build_fields_msgpack(Some(r#"{"files":[]}"#)), vec![0x80]);
}

// ── both fields together ──────────────────────────────────────────────────────

#[test]
fn image_and_files_fixmap2() {
    let json = format!(
        r#"{{"image":{{"mimeType":"image/png","data":"{}"}},"files":[{{"name":"x","data":"{}"}}]}}"#,
        B64.encode(b"png"), B64.encode(b"file"),
    );
    let mp = build_fields_msgpack(Some(&json));
    assert_eq!(mp[0] & 0xf0, 0x80);
    assert_eq!(mp[0] & 0x0f, 2, "two fields");
}

// ── full encode → decode roundtrips ──────────────────────────────────────────

#[test]
fn image_roundtrip() {
    let img = b"\xff\xd8\xff\xe0fake_jpeg_bytes";
    let json = format!(r#"{{"image":{{"mimeType":"image/jpeg","data":"{}"}}}}"#, B64.encode(img));
    let dec = decode_lxmf_payload(&wire_with_media(&json)).unwrap();
    let (mime, data) = dec.image.expect("image present");
    assert_eq!(mime, "image/jpeg");
    assert_eq!(data, img);
}

#[test]
fn image_png_roundtrip() {
    let img = b"\x89PNG\r\n\x1a\nfake";
    let json = format!(r#"{{"image":{{"mimeType":"image/png","data":"{}"}}}}"#, B64.encode(img));
    let dec = decode_lxmf_payload(&wire_with_media(&json)).unwrap();
    assert_eq!(dec.image.unwrap().0, "image/png");
}

#[test]
fn single_file_roundtrip() {
    let content = b"PDF content here";
    let json = format!(r#"{{"files":[{{"name":"report.pdf","data":"{}"}}]}}"#, B64.encode(content));
    let dec = decode_lxmf_payload(&wire_with_media(&json)).unwrap();
    assert_eq!(dec.files.len(), 1);
    assert_eq!(dec.files[0].0, "report.pdf");
    assert_eq!(dec.files[0].1, content);
}

#[test]
fn multiple_files_roundtrip_preserves_order() {
    let files: &[(&str, &[u8])] = &[
        ("photo.jpg", b"\xff\xd8jpeg"),
        ("notes.txt", b"some notes"),
        ("data.bin",  &[0x00, 0xff, 0x7f, 0x80]),
    ];
    let json = format!(
        r#"{{"files":[{}]}}"#,
        files.iter().map(|(n, d)| format!(r#"{{"name":"{}","data":"{}"}}"#, n, B64.encode(d)))
             .collect::<Vec<_>>().join(",")
    );
    let dec = decode_lxmf_payload(&wire_with_media(&json)).unwrap();
    assert_eq!(dec.files.len(), 3);
    for (i, (name, data)) in files.iter().enumerate() {
        assert_eq!(dec.files[i].0, *name);
        assert_eq!(dec.files[i].1, *data);
    }
}

#[test]
fn full_media_image_plus_files_roundtrip() {
    let json = format!(
        r#"{{"image":{{"mimeType":"image/gif","data":"{}"}},"files":[{{"name":"a.txt","data":"{}"}},{{"name":"b.bin","data":"{}"}}]}}"#,
        B64.encode(b"gif_bytes"),
        B64.encode(b"text"),
        B64.encode(b"\x00\x01binary"),
    );
    let dec = decode_lxmf_payload(&wire_with_media(&json)).unwrap();
    assert_eq!(dec.image.as_ref().unwrap().0, "image/gif");
    assert_eq!(dec.files.len(), 2);
    assert_eq!(dec.files[0].0, "a.txt");
    assert_eq!(dec.files[1].0, "b.bin");
}

#[test]
fn body_preserved_alongside_image() {
    let json = format!(r#"{{"image":{{"mimeType":"image/jpeg","data":"{}"}}}}"#, B64.encode(b"img"));
    let mut w = vec![0u8; 96];
    w.extend_from_slice(&encode_lxmf_msgpack(
        0.0, b"my title", b"my body",
        &build_fields_msgpack(Some(&json)),
    ));
    let dec = decode_lxmf_payload(&w).unwrap();
    assert_eq!(dec.title, b"my title");
    assert_eq!(dec.body,  b"my body");
    assert!(dec.image.is_some());
}
