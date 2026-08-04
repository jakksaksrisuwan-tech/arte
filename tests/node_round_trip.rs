//! Round-trip tests for the canonical Node model.
//! These guard the contract: any node, parsed and re-serialized, produces the
//! same content. Unknown keys survive. Multi-value fields survive in order.

use arte::{Node, flag_value, has_flag, load_runs, stable_pass, staleness, write_run, RunRec};

#[test]
fn parse_to_text_is_stable() {
    // Idempotent: parse → serialize → reparse → serialize → identical
    let raw = "id: v1\nrole: validation\nserves: c1\nserves: c2\nweird: keep-me\n";
    let n = Node::parse(raw);
    assert_eq!(n.get("role"), Some("validation"));
    assert_eq!(n.all("serves"), vec!["c1", "c2"]);
    assert_eq!(n.get("weird"), Some("keep-me")); // unknown key survives
    let again = Node::parse(&n.to_text());
    assert_eq!(again.to_text(), n.to_text()); // round-trip stable
}

#[test]
fn field_mutators_replace_and_append() {
    let mut n = Node::parse("role: impl\nserves: a\nserves: b");
    n.set_field("role", "control"); // replace scalar
    assert_eq!(n.get("role"), Some("control"));
    n.push_field("serves", "a"); // dup skipped
    assert_eq!(n.all("serves").len(), 2);
    assert!(n.remove_value("serves", "a"));
    assert_eq!(n.all("serves"), vec!["b"]);
    assert!(n.unset_field("serves"));
    assert!(n.all("serves").is_empty());
    assert!(!n.unset_field("serves")); // nothing left → false
}

#[test]
fn parse_ignores_blank_and_comment_lines() {
    let raw = "# a comment\n\nid: c1\n# another\nrole: control\n\ntitle: t\n";
    let n = Node::parse(raw);
    assert_eq!(n.get("id"), Some("c1"));
    assert_eq!(n.get("role"), Some("control"));
    assert_eq!(n.get("title"), Some("t"));
}

#[test]
fn parse_keeps_first_value_after_first_colon() {
    // values may contain `:` — first colon splits
    let n = Node::parse("at: src/auth.rs#hash_password");
    assert_eq!(n.get("at"), Some("src/auth.rs#hash_password"));
}

#[test]
fn flag_value_returns_arg_after_dashdash_name() {
    let args: Vec<String> = ["arte", "status", "v1", "ok", "--requires-stable-pass", "--force"]
        .iter().map(|s| s.to_string()).collect();
    assert_eq!(flag_value(&args, "--requires-stable-pass"), Some("--force".to_string()));
    assert_eq!(flag_value(&args, "--missing"), None);
    // last arg as flag value: returns None (no value after)
    let last: Vec<String> = ["a", "--end"].iter().map(|s| s.to_string()).collect();
    assert_eq!(flag_value(&last, "--end"), None);
}

#[test]
fn has_flag_detects_boolean_flags() {
    let args: Vec<String> = ["arte", "cycle", "--once", "--forever"]
        .iter().map(|s| s.to_string()).collect();
    assert!(has_flag(&args, "--once"));
    assert!(has_flag(&args, "--forever"));
    assert!(!has_flag(&args, "--missing"));
}

#[test]
fn stable_pass_thresholds() {
    let mk = |result: &str, seq: &str| RunRec {
        seq: seq.to_string(), result: result.to_string(),
        sha: String::new(), timestamp: String::new(), note: String::new(),
    };
    // empty: trivially 0 of 0 → stable (boundary — caller checks len)
    let empty: Vec<RunRec> = vec![];
    let (p, s) = stable_pass(&empty);
    assert_eq!(p, 0);
    assert!(!s);

    // 1 pass: window=1, need 2 → not stable
    let one = vec![mk("pass", "001")];
    assert_eq!(stable_pass(&one), (1, false));

    // 2 of 5 pass: meets threshold
    let mut five = vec![mk("pass","005"), mk("pass","004"), mk("fail","003"), mk("fail","002"), mk("fail","001")];
    assert_eq!(stable_pass(&five), (2, true));
    // ordering doesn't matter — window is the last 5 (i.e. all of them)
    five.reverse();
    assert_eq!(stable_pass(&five), (2, true));

    // 1 of 5 pass: below
    let low: Vec<RunRec> = vec![mk("fail","005"), mk("fail","004"), mk("fail","003"), mk("fail","002"), mk("pass","001")];
    assert_eq!(stable_pass(&low), (1, false));

    // window grows past 5: still uses last 5
    let long = vec![
        mk("pass","006"), mk("pass","005"), mk("pass","004"),
        mk("fail","003"), mk("fail","002"), mk("fail","001"),
    ];
    assert_eq!(stable_pass(&long), (3, true));
}

#[test]
fn staleness_returns_none_when_no_sha() {
    let n = Node::parse("id: v1\nrole: validation");
    let mut cache = std::collections::HashMap::new();
    assert!(staleness(&n, &mut cache).is_none());
}

#[test]
fn write_run_then_load_round_trips() {
    // write_run auto-increments seq — write two, then load
    let rec = RunRec {
        seq: String::new(), result: "pass".into(),
        sha: "ffbb897".into(), timestamp: "2026-08-04T10:00:00Z".into(),
        note: "test".into(),
    };
    write_run("__t_test_v1", &rec).expect("write run");
    let loaded = load_runs("__t_test_v1");
    assert!(!loaded.is_empty());
    assert_eq!(loaded[0].result, "pass");
    assert_eq!(loaded[0].sha, "ffbb897");
    assert_eq!(loaded[0].note, "test");
    // cleanup
    let dir = std::path::Path::new("qa/runs");
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with("__t_test_v1.") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}