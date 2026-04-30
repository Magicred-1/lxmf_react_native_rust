use crate::node::parse_interfaces_json;

// ── valid inputs ──────────────────────────────────────────────────────────────

#[test]
fn single_interface_parsed() {
    let r = parse_interfaces_json(r#"[{"host":"127.0.0.1","port":4242}]"#).unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].0, "127.0.0.1");
    assert_eq!(r[0].1, 4242);
}

#[test]
fn multiple_interfaces_all_parsed() {
    let json = r#"[{"host":"a.example.com","port":1234},{"host":"b.example.com","port":5678}]"#;
    let r = parse_interfaces_json(json).unwrap();
    assert_eq!(r.len(), 2);
    assert_eq!(r[0], ("a.example.com".to_string(), 1234));
    assert_eq!(r[1], ("b.example.com".to_string(), 5678));
}

#[test]
fn empty_array_returns_empty_vec() {
    let r = parse_interfaces_json("[]").unwrap();
    assert!(r.is_empty());
}

#[test]
fn port_min_boundary_1_valid() {
    let r = parse_interfaces_json(r#"[{"host":"h","port":1}]"#).unwrap();
    assert_eq!(r[0].1, 1);
}

#[test]
fn port_max_boundary_65535_valid() {
    let r = parse_interfaces_json(r#"[{"host":"h","port":65535}]"#).unwrap();
    assert_eq!(r[0].1, 65535);
}

#[test]
fn ipv6_host_accepted() {
    let r = parse_interfaces_json(r#"[{"host":"::1","port":4242}]"#).unwrap();
    assert_eq!(r[0].0, "::1");
}

// ── error conditions ──────────────────────────────────────────────────────────

#[test]
fn invalid_json_returns_err() {
    assert!(parse_interfaces_json("not json").is_err());
}

#[test]
fn missing_host_returns_err() {
    assert!(parse_interfaces_json(r#"[{"port":1234}]"#).is_err());
}

#[test]
fn empty_host_returns_err() {
    assert!(parse_interfaces_json(r#"[{"host":"","port":1234}]"#).is_err());
}

#[test]
fn missing_port_returns_err() {
    assert!(parse_interfaces_json(r#"[{"host":"h"}]"#).is_err());
}

#[test]
fn port_zero_returns_err() {
    assert!(parse_interfaces_json(r#"[{"host":"h","port":0}]"#).is_err());
}

#[test]
fn port_too_large_returns_err() {
    assert!(parse_interfaces_json(r#"[{"host":"h","port":65536}]"#).is_err());
}

#[test]
fn port_negative_returns_err() {
    assert!(parse_interfaces_json(r#"[{"host":"h","port":-1}]"#).is_err());
}

#[test]
fn extra_fields_ignored() {
    let r = parse_interfaces_json(r#"[{"host":"h","port":80,"extra":"ignored","nested":{}}]"#).unwrap();
    assert_eq!(r[0], ("h".to_string(), 80));
}

#[test]
fn error_message_includes_index() {
    let err = parse_interfaces_json(r#"[{"host":"h","port":1},{"host":"","port":2}]"#).unwrap_err();
    assert!(err.contains('1') || err.contains("Interface 1"), "error should identify failing index: {err}");
}
