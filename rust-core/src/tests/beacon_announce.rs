use crate::node::build_app_data;

const PREFIX: &[u8] = b"anonmesh::beacon::v1\0";

// ── non-beacon mode ───────────────────────────────────────────────────────────

#[test]
fn non_beacon_returns_display_name() {
    let d = build_app_data("Alice", false);
    assert_eq!(d, b"Alice");
}

#[test]
fn non_beacon_empty_name_defaults_to_lxmf_mobile() {
    let d = build_app_data("", false);
    assert_eq!(d, b"lxmf-mobile");
}

#[test]
fn non_beacon_name_truncated_to_32_bytes() {
    let long = "a".repeat(50);
    let d = build_app_data(&long, false);
    assert_eq!(d.len(), 32);
    assert_eq!(d, b"a".repeat(32).as_slice());
}

// ── beacon mode ───────────────────────────────────────────────────────────────

#[test]
fn beacon_starts_with_prefix() {
    let d = build_app_data("Alice", true);
    assert!(d.starts_with(PREFIX), "must start with beacon prefix");
}

#[test]
fn beacon_contains_display_name_after_prefix() {
    let d = build_app_data("Bob", true);
    assert_eq!(&d[PREFIX.len()..], b"Bob");
}

#[test]
fn beacon_empty_name_defaults_to_lxmf_mobile() {
    let d = build_app_data("", true);
    assert!(d.starts_with(PREFIX));
    assert_eq!(&d[PREFIX.len()..], b"lxmf-mobile");
}

#[test]
fn beacon_name_truncated_to_32_bytes() {
    let long = "x".repeat(50);
    let d = build_app_data(&long, true);
    assert!(d.starts_with(PREFIX));
    assert_eq!(&d[PREFIX.len()..], b"x".repeat(32).as_slice());
}

#[test]
fn beacon_prefix_contains_null_separator() {
    assert_eq!(PREFIX[PREFIX.len() - 1], 0, "prefix ends with \\0 separator");
}

#[test]
fn beacon_total_length_is_prefix_plus_name() {
    let name = "MyNode";
    let d = build_app_data(name, true);
    assert_eq!(d.len(), PREFIX.len() + name.len());
}

// ── CLI startswith compatibility ──────────────────────────────────────────────

#[test]
fn cli_can_detect_beacon_via_startswith() {
    let d = build_app_data("SomeNode", true);
    assert!(d.starts_with(b"anonmesh::beacon::v1"));
}

#[test]
fn non_beacon_fails_cli_detection() {
    let d = build_app_data("SomeNode", false);
    assert!(!d.starts_with(b"anonmesh::beacon::v1"));
}
