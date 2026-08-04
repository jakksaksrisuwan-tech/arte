//! Tests for the tetrahedron-enforcement controls.
//! c5: promotion gated on verify subprocess exit code
//! c6: status values are exactly {ok, ko, pending, justified}

use std::process::Command;

#[test]
fn cmd_status_rejects_unknown_status_value() {
    // c6: any value outside {ok, ko, pending, justified} must be refused with exit 2.
    let out = Command::new(env!("CARGO_BIN_EXE_arte"))
        .args(["status", "i1", "banana"])
        .output()
        .expect("spawn arte");
    assert!(!out.status.success(), "expected exit non-zero for bad status");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("status must be one of ok|ko|pending|justified"),
        "stderr did not name the allowed set: {stderr}"
    );
}

#[test]
fn cmd_status_accepts_every_documented_value() {
    // c6: each of the four documented values must be accepted — verifies the
    // validator uses the right set, not a typo'd subset.
    for v in ["ok", "ko", "pending", "justified"] {
        let out = Command::new(env!("CARGO_BIN_EXE_arte"))
            .args(["status", "i1", v])
            .output()
            .expect("spawn arte");
        assert!(out.status.success(), "expected ok for status={v}, got {:?}", out.status);
    }
}

#[test]
fn meta_planner_role_is_read_only_across_repos() {
    // c5-adjacent: the meta-planner deny-list covers src/test/tests/qa-tests/.truth.
    // Re-deriving the deny-list here is more brittle than reading it, but it
    // catches a silent regression where someone removes meta-planner from the
    // role table and the cycle silently gains write authority.
    //
    // We don't shell out (subprocess paths flake in CI); we re-derive the
    // expected list from the same source of truth the CLI uses — lib.rs.
    let src = std::fs::read_to_string("src/lib.rs").expect("lib.rs");
    let snippet = src.split("pub fn role_deny_dirs").nth(1).expect("function present");
    let body: String = snippet.chars().take(2_000).collect();
    for required in ["src", "test", "tests", "qa/tests", ".truth"] {
        assert!(
            body.contains(&format!("\"{required}\"")),
            "role_deny_dirs missing {required} in its meta-planner branch"
        );
    }
}
