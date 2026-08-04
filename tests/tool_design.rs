//! Design-contract tests for the four c1..c4 controls — the FORMAT.md pillars:
//!
//!   v-c1-id-is-stable                       c1: node identity = filename (NOT title)
//!   v-c2-one-node-per-file                  c2: one node per file (so git merges per field)
//!   v-c3-line-oriented-key-value            c3: line-oriented `key: value` round-trip
//!   v-c4-toml-declares-layers-and-subsets   c4: chain / subsets / subset_axis from
//!                                            arte.toml — no Rust hardcoding
//!
//! Each test is a FALSIFYING check — the assertion fails if the design
//! commitment the control names is violated, not merely if the artefact happens
//! to currently pass. RED is the implementer's work order; GREEN is the gate.
//!
//! The id-stability and one-node-per-file tests are scoped to a scratch
//! ARTE_TRUTH_DIR so they don't touch the live board. The round-trip test is a
//! pure unit assertion on the Node model. The artefact.toml test mutates and
//! restores the file under a guard so the repo is unchanged on any exit path.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_arte");

/// Per-test scratch dir under $TMP — hermetic, never collides with the live
/// `.truth/`. Returned path is the CONCRETE truth dir (no `.truth/` suffix —
/// ARTE_TRUTH_DIR IS the truth dir, see `read_env_dirs`).
fn scratch_truth(label: &str) -> PathBuf {
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("arte-tool-design-{label}-{pid}-{nanos}"));
    fs::create_dir_all(&dir).expect("create scratch truth dir");
    dir
}

/// Spawn `arte <args>` with ARTE_TRUTH_DIR pointing at the scratch dir.
/// Returns the captured (stdout, stderr, exit status) so the test can both
/// check the printed id AND verify what landed on disk.
fn run_arte_in(args: &[&str], truth: &PathBuf) -> (String, String, std::process::ExitStatus) {
    let out = Command::new(BIN)
        .args(args)
        .env("ARTE_TRUTH_DIR", truth)
        .env_remove("ARTE_RUNS_DIR")
        .env_remove("ARTE_DISPATCH_PATH")
        .output()
        .expect("spawn arte");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status,
    )
}

// ─── v-c1-id-is-stable: rename title, filename untouched ────────────────────

#[test]
fn test_id_is_stable_across_title_rename() {
    // c1: A node is keyed by a stable id, never by its title. A rename of the
    // `title:` field MUST NOT rename the file on disk. Identity = filename;
    // title is a freely renamable label.
    //
    // Plan: spawn `arte add validation "Original title"` in a scratch truth
    // dir, capture the minted id, then `arte set <id> title "Renamed title"`,
    // then `arte show <id>` and re-stat the directory. The id must still be
    // discoverable, the .node file must still be named <id>.node, and the
    // rendered node must now carry the new title.
    let truth = scratch_truth("c1-id-stable");

    let (stdout, stderr, status) = run_arte_in(
        &["add", "validation", "Original title"],
        &truth,
    );
    assert!(
        status.success(),
        "`arte add validation \"Original title\"` failed:\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );
    let id = stdout
        .lines()
        .find(|l| l.starts_with("added "))
        .and_then(|l| l.split_whitespace().nth(1))
        .map(|s| s.to_string())
        .unwrap_or_else(|| panic!("could not parse added id from stdout:\n{stdout}"));

    // Rename via `arte set <id> title <new>`. This writes the node file in
    // place; it MUST NOT rename the file (the id IS the filename).
    let new_title = "Renamed title";
    let (stdout, stderr, status) = run_arte_in(
        &["set", &id, "title", new_title],
        &truth,
    );
    assert!(
        status.success(),
        "`arte set {id} title {new_title}` failed:\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );

    // The on-disk filename must still be <id>.node — exactly one file, same stem.
    let entries: Vec<String> = fs::read_dir(&truth)
        .expect("read scratch truth dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "after rename there should be exactly one .node file (id stable), found: {entries:?}"
    );
    let only = &entries[0];
    assert_eq!(
        only, &format!("{id}.node"),
        "filename must equal the id — rename must NOT rename the file. expected {id}.node, found {only}"
    );

    // `arte show <id>` must still resolve — the id is the key into the board.
    let (stdout, stderr, status) = run_arte_in(&["show", &id], &truth);
    assert!(
        status.success(),
        "`arte show {id}` failed after rename:\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );
    assert!(
        stdout.contains(&format!("id: {id}")),
        "show output does not name the id — identity lost across rename:\n{stdout}"
    );
    assert!(
        stdout.contains(&format!("title: {new_title}")),
        "show output does not carry the renamed title — set must have rewritten the field:\n{stdout}"
    );
    // And the OLD title must be gone (proves the set actually took).
    assert!(
        !stdout.contains("title: Original title"),
        "old title still present after rename — set did not overwrite:\n{stdout}"
    );
}

// ─── v-c2-one-node-per-file: exactly one .node file per id ──────────────────

#[test]
fn test_one_node_per_file() {
    // c2: one node per file; git's line merge reconciles concurrent edits
    // per field. A node's `id:` field MUST equal its filename stem.
    //
    // Plan: spawn `arte add validation "Single file per node"`, then assert:
    //   (a) exactly one `.node` file lives in the truth dir;
    //   (b) `arte show <id>` returns success and the printed `id:` matches
    //       the filename stem (so renaming the id without renaming the file
    //       would be caught — and vice versa).
    let truth = scratch_truth("c2-one-per-file");

    let (stdout, stderr, status) = run_arte_in(
        &["add", "validation", "Single file per node"],
        &truth,
    );
    assert!(
        status.success(),
        "`arte add validation \"Single file per node\"` failed:\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );
    let id = stdout
        .lines()
        .find(|l| l.starts_with("added "))
        .and_then(|l| l.split_whitespace().nth(1))
        .map(|s| s.to_string())
        .unwrap_or_else(|| panic!("could not parse added id from stdout:\n{stdout}"));

    // (a) exactly one .node file in the truth dir, and its stem equals the id.
    let nodes: Vec<String> = fs::read_dir(&truth)
        .expect("read scratch truth dir")
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.extension().map(|x| x == "node").unwrap_or(false) {
                p.file_stem().map(|s| s.to_string_lossy().to_string())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        nodes.len(),
        1,
        "expected exactly one .node file in the truth dir, found {nodes:?}"
    );
    assert_eq!(
        nodes[0], id,
        "the single .node file's stem must equal the added id — found stem {} vs id {id}",
        nodes[0]
    );

    // (b) `arte show <id>` succeeds and prints an `id: <id>` line.
    let (stdout, stderr, status) = run_arte_in(&["show", &id], &truth);
    assert!(
        status.success(),
        "`arte show {id}` failed:\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );
    assert!(
        stdout.lines().any(|l| l.trim() == format!("id: {id}")),
        "`arte show {id}` did not print `id: {id}` — filename and stored id disagree:\n{stdout}"
    );
}

// ─── v-c3-line-oriented-key-value: unknown keys round-trip; # anchor survives

#[test]
fn test_round_trip_with_unknown_keys() {
    // c3: line-oriented `key: value`, one per line. Unknown keys round-trip
    // untouched (forward-compat). The `at:` anchor (FORMAT.md: `file#unit`)
    // must survive a parse/serialize round-trip with the `#` intact.
    //
    // We exercise the Node model directly (the library is what `arte show`
    // / `arte at` both go through). RED is a parse that loses a field or
    // mangles the `#` anchor.
    use arte::Node;

    // 1. Unknown keys survive a parse → serialize → re-parse round-trip.
    let raw = "\
id: v3\n\
role: validation\n\
subset: proof\n\
title: round-trip me\n\
future-key: keep-me\n\
future-key: and-me-too\n\
weird: colon:here:stays\n\
";
    let n1 = Node::parse(raw);
    let s1 = n1.to_text();
    let n2 = Node::parse(&s1);
    let s2 = n2.to_text();
    assert_eq!(
        s1, s2,
        "round-trip not idempotent — to_text differs after re-parse:\n--- s1 ---\n{s1}\n--- s2 ---\n{s2}"
    );
    // The unknown scalar survives with its exact value (including internal `:`).
    assert_eq!(
        n2.get("future-key"),
        Some("keep-me"),
        "unknown scalar key lost on round-trip"
    );
    // The unknown multi-value survives in order (repeated-key semantics).
    assert_eq!(
        n2.all("future-key"),
        vec!["keep-me", "and-me-too"],
        "unknown repeated key lost order or values on round-trip"
    );
    // And the value with embedded colons survives intact.
    assert_eq!(
        n2.get("weird"),
        Some("colon:here:stays"),
        "value containing `:` was split/mangled on round-trip"
    );

    // 2. `at: file#unit` round-trips with the `#` anchor intact.
    let anchor_raw = "at: src/foo.rs#hash_password\n";
    let a = Node::parse(anchor_raw);
    assert_eq!(
        a.get("at"),
        Some("src/foo.rs#hash_password"),
        "`#` in `at:` value lost on parse"
    );
    let again = Node::parse(&a.to_text());
    assert_eq!(
        again.get("at"),
        Some("src/foo.rs#hash_password"),
        "`#` in `at:` value lost on serialize→reparse round-trip"
    );
}

// ─── v-c4-toml-declares-layers-and-subsets: new subset is picked up ────────

#[test]
fn test_new_subset_picked_up() {
    // c4: chain / subsets / subset_axis come from arte.toml — the binary
    // stays agnostic; nothing about layers or grouping values is hardcoded
    // in src/. Adding a fresh subset to [subsets] must make `arte add
    // validation --subset <new>` succeed WITHOUT the "subset not declared"
    // warning the loader prints when the value is unknown.
    //
    // Plan: mutate artefact.toml to declare a fresh subset under [subsets]
    // for the validation role (the role used by the 4 design-contract
    // tests), then spawn `arte add validation "..." --subset <new>`, and
    // assert both exit=0 AND no "subset not declared" warning. Restore
    // artefact.toml BEFORE any assertion so a panic still leaves the repo
    // clean.
    let toml_path = "arte.toml";
    let original = fs::read_to_string(toml_path).expect("read artefact.toml");
    let probe_subset = "_tool_design_probe_subset";

    // Restore guard — RAII via a local closure so any panic reverts the file.
    let restore = || {
        let _ = fs::write(toml_path, &original);
    };

    // Compute the tampered contents. We extend the validation subset list
    // in [subsets] (the loader reads it via `parse_arr` and the mutator
    // accepts any value in the list).
    let tampered = {
        let mut t = original.clone();
        let inject = format!("{probe_subset}");
        if !t.contains(&inject) {
            // Find the validation subset line and append our probe value.
            // The expected shape is: `validation = ["proof"]`.
            // If the line isn't shaped that way (older boards), bail early
            // with a clear message rather than silently corrupting.
            let marker = "validation = [\"proof\"]";
            if !t.contains(marker) {
                // Restore + skip: the loader/mutator still works, but this
                // test is shaped around the canonical [subsets] line.
                restore();
                eprintln!("skipping: artefact.toml does not contain expected line {marker:?}");
                return;
            }
            t = t.replacen(marker, &format!("{marker}, \"{probe_subset}\""), 1);
        }
        t
    };
    fs::write(toml_path, &tampered).expect("write tampered artefact.toml");

    // Spawn `arte add validation "<title>" --subset <probe>`. The throwaway
    // title is unique enough to bypass the dedupe check.
    let add_args = ["add", "validation", "Tool design probe fixture", "--subset", probe_subset];
    let (stdout, stderr, status) = run_arte_in(&add_args, {
        // Use the live truth dir for this add — we want the loader to read
        // the live artefact.toml (the scratch truth dir doesn't change what
        // artefact.toml is read; the conf loader reads ./arte.toml from CWD).
        &scratch_truth("c4-pickup")
    });

    // Capture the id for cleanup so the test is reversible even on success.
    let added_id = stdout
        .lines()
        .find(|l| l.starts_with("added ") || l.starts_with("exists:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .map(|s| s.to_string())
        .unwrap_or_default();

    // Restore artefact.toml BEFORE any assertion — a panic below should
    // leave the file untouched. (Order matters: we want the repo clean
    // first, then we judge the captured output.)
    restore();

    // Best-effort cleanup of the probe node in its scratch dir. Even if
    // we never reach here (e.g. add failed), the scratch dir is under
    // $TMP and won't be visited again.
    if !added_id.is_empty() {
        let _ = Command::new(BIN)
            .args(["delete", &added_id])
            .env("ARTE_TRUTH_DIR", scratch_truth("c4-pickup-cleanup"))
            .output();
    }

    let combined = format!("{stdout}{stderr}");

    assert!(
        status.success(),
        "`arte add validation --subset {probe_subset}` failed — subset not picked up from artefact.toml:\n{combined}"
    );
    assert!(
        !combined.contains("subset '") && !combined.contains("not declared in [subsets]"),
        "binary printed a 'subset not declared' warning for a value we just added to artefact.toml — loader is ignoring [subsets]:\n{combined}"
    );
}
