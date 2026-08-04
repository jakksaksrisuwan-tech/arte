//! Edge-case tests for things the prior reviews flagged: panic traps,
//! silent-error swallowing, lossy string parsing. These are regression
//! guards for bugs that wouldn't surface in a happy-path smoke test.

use arte::{Node, trunc};

#[test]
fn trunc_with_zero_width_does_not_panic() {
    // The original `s.chars().take(n - 1)` underflowed when n=0 and the
    // input was longer than 0 chars. Should return empty, not panic.
    assert_eq!(trunc("hello world", 0), "");
    assert_eq!(trunc("", 0), "");
}

#[test]
fn trunc_with_short_input_pads() {
    assert_eq!(trunc("hi", 5), "hi   ");
    assert_eq!(trunc("hello", 5), "hello");
}

#[test]
fn trunc_with_long_input_truncates_with_ellipsis() {
    let t = trunc("hello world this is long", 10);
    // 9 chars + ellipsis = 10 visible chars (or close to it)
    assert!(t.ends_with('…'));
    assert!(t.chars().count() <= 10);
}

#[test]
fn node_round_trip_preserves_field_order_and_unknown_keys() {
    // Forward-compat: future fields must not be lost on read.
    let raw = "id: v1\nrole: validation\nfuture_field: future-value\nserves: c1\nserves: c2\n";
    let n = Node::parse(raw);
    let text = n.to_text();
    assert!(text.contains("future_field: future-value"));
    assert!(text.contains("serves: c1"));
    assert!(text.contains("serves: c2"));
    // Re-parse and verify multi-value preserved
    let again = Node::parse(&text);
    assert_eq!(again.all("serves"), vec!["c1", "c2"]);
    assert_eq!(again.get("future_field"), Some("future-value"));
}

#[test]
fn parse_handles_crlf_and_unix_line_endings() {
    let raw_crlf = "id: v1\r\nrole: validation\r\n";
    let n = Node::parse(raw_crlf);
    assert_eq!(n.get("id"), Some("v1"));
    assert_eq!(n.get("role"), Some("validation"));
}

#[test]
fn parse_tolerates_trailing_whitespace_and_comments() {
    let raw = "id: v1   \n# this is a comment\nrole: validation\n";
    let n = Node::parse(raw);
    assert_eq!(n.get("id"), Some("v1"));
    assert_eq!(n.get("role"), Some("validation"));
}