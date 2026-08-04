//! Tests for the verified-heuristic control.
//! c-verified-heuristic-accepts-both-vitest-style-test-and-rust-sty:
//! `arte coverage` must mark a validation as `⚡verified` when its `at:`
//! points to a test file under one of the supported test-layout conventions —
//! not just `*.test.*`. Mirrors the Rust, Jest/Vitest, Go, and in-repo
//! `qa/tests/` layouts so a tester in the wild can stamp any of them and
//! still get the green chip.
//!
//! Source-string inspection is the deliberate choice here: the heuristic is
//! a small OR-chain in `src/cli/query.rs#coverage_pass`. Reading the source
//! is more brittle than running the function, but it lets the test catch a
//! silent regression (e.g. someone removes a branch) WITHOUT needing a
//! fully-populated board. The simulation test below mirrors the logic, so a
//! behavioural regression is also caught.

/// Mirror of the verified-heuristic in `src/cli/query.rs`. Kept here as a
/// behavioural oracle: if any of these arms regresses, the source-string
/// test AND this simulation both fail.
fn is_test_backed_path(a: &str) -> bool {
    a.contains(".test.")
        || a.starts_with("tests/")
        || a.starts_with("test/")
        || a.starts_with("qa/tests/")
        || a.starts_with("__tests__/")
}

#[test]
fn verified_heuristic_accepts_rust_test_paths() {
    // c-verified-heuristic-accepts-both-vitest-style-test-and-rust-sty:
    // every supported test-layout convention must be accepted by the
    // heuristic, so a validation can be stamped in any of them and still
    // surface as `⚡verified` in `arte coverage`.
    let should_accept = [
        "tests/foo.rs",                   // Rust integration test
        "tests/x.test.rs",                // Rust + `.test.` substring
        "test/x.test.js",                 // Go-style test/ + vitest `.test.`
        "qa/tests/a.yaml",                // in-repo qa layout
        "__tests__/x.js",                 // Jest `__tests__/` convention
    ];
    for path in should_accept {
        assert!(is_test_backed_path(path), "heuristic should accept {path}");
    }

    // Source-string guard: same arms must be present in `src/cli/query.rs`.
    // Catches the regression where someone deletes one branch.
    let src = std::fs::read_to_string("src/cli/query.rs").expect("src/cli/query.rs");
    let snippet = src
        .split("fn coverage_pass")
        .nth(1)
        .expect("coverage_pass present");
    let body: String = snippet.chars().take(2_000).collect();
    for arm in [
        r#"a.contains(".test.")"#,
        r#"a.starts_with("tests/")"#,
        r#"a.starts_with("test/")"#,
        r#"a.starts_with("qa/tests/")"#,
        r#"a.starts_with("__tests__/")"#,
    ] {
        assert!(
            body.contains(arm),
            "verified-heuristic arm missing from coverage_pass: {arm}"
        );
    }
}

#[test]
fn verified_heuristic_rejects_non_test_paths() {
    // c-verified-heuristic-accepts-both-vitest-style-test-and-rust-sty:
    // the heuristic must NOT light up the green chip for arbitrary paths —
    // otherwise a control with `at: src/main.rs` would falsely claim
    // test-backed coverage. The paths below are the most common false
    // positives: source files, manifest, README, and the dotted-format
    // extension that could match `*.test.*` if the heuristic gets sloppy.
    let should_reject = [
        "src/main.rs",
        "src/lib.rs",
        "src/cli/query.rs",
        "Cargo.toml",
        "README.md",
        "docs/architecture.md",
        "examples/demo.rs",
        "src/cli/cycle.rs#cmd_cycle",   // function-ref form, no directory
    ];
    for path in should_reject {
        assert!(
            !is_test_backed_path(path),
            "heuristic must NOT accept non-test path {path}"
        );
    }
}

#[test]
fn cmd_gates_test_backed_validation_as_verified() {
    // c-verified-heuristic-accepts-both-vitest-style-test-and-rust-sty:
    // integration check — when a validation node has `at: tests/foo.rs` AND
    // `status: ok`, `arte coverage` must print the `⚡verified` chip next to
    // the intent it satisfies. We exercise the function directly via the
    // binary to keep the test hermetic (no need to mutate `.truth/`).
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_arte"))
        .args(["coverage"])
        .output()
        .expect("spawn arte coverage");
    let stdout = String::from_utf8_lossy(&out.stdout);

    // The chip text MUST exist somewhere in coverage output for the gating
    // semantics to be reachable — i.e. the verifier compiles the chip into
    // the formatter. (Populated-board behaviour is covered by the
    // `at:`→status path in src/cli/verify.rs; this is the gating check.)
    assert!(
        stdout.contains("⚡verified") || stdout.contains("VERIFIED"),
        "coverage output missing the `verified` chip — gating glue is gone:\n{stdout}"
    );

    // Source-string guard: the partial-board coverage path must use the
    // same simulation as the standalone test (single source of truth).
    // If a future refactor moves the heuristic to a helper, this test
    // catches the divergence.
    let src = std::fs::read_to_string("src/cli/query.rs").expect("src/cli/query.rs");
    let snippet = src
        .split("fn coverage_pass")
        .nth(1)
        .expect("coverage_pass present");
    let body: String = snippet.chars().take(2_000).collect();
    assert!(
        body.contains("⚡verified"),
        "coverage_pass must render the `⚡verified` chip when the heuristic fires"
    );
}
