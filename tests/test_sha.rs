//! v-test-sha-is-the-git-blob-sha-1-of-the-test-file-and-drift-is-d
//! c-verify-records-the-sha-1-of-the-test-artifact-and-staleness-is
//!
//! `sha:` records git HEAD at verify time — useless when the test artifact is
//! not in git. Measured on the subject repo: its whole spec suite lives in a
//! gitignored `qa/tests/`, so specs were edited dozens of times while every
//! node's `sha:` stayed put; a green said nothing about the test that produced
//! it. `test_sha:` closes that: the git-blob SHA-1 of the `at:` file, so the
//! same digest is reproducible with `git hash-object <file>` even for files
//! git does not track.
//!
//! FALSIFYING: known SHA-1 vectors must match; the blob form must equal
//! `git hash-object`; and changed test content must read as drifted.

use std::fs;
use std::process::Command;

use arte::{git_blob_sha1, git_head_sha1, sha1_hex, test_sha_drift};

#[test]
fn sha1_matches_known_vectors() {
    // FIPS 180-1 / RFC 3174 published vectors — if the implementation is wrong
    // these are the first things to move.
    assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    assert_eq!(
        sha1_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
    );
    // a length that crosses the 64-byte block boundary (padding path)
    assert_eq!(sha1_hex(&[b'a'; 1000000][..1000]), "291e9a6c66994949b57ba5e650361e98fc36b1ba");
}

#[test]
fn blob_form_equals_git_hash_object() {
    let dir = std::env::temp_dir().join(format!("arte-testsha-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("scratch");
    let f = dir.join("probe.txt");
    fs::write(&f, b"arte tested-sha probe\n").expect("write");
    let ours = git_blob_sha1(&f).expect("hash file");
    let theirs = Command::new("git")
        .args(["hash-object", f.to_str().unwrap()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string());
    if let Some(t) = theirs {
        assert_eq!(
            ours, t,
            "test_sha must be reproducible with `git hash-object` — that is what makes it \
             auditable for files git does not track"
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn changed_test_content_reads_as_drifted() {
    let dir = std::env::temp_dir().join(format!("arte-testdrift-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("scratch");
    let f = dir.join("spec.yaml");
    fs::write(&f, b"one: 1\n").expect("write");
    let recorded = git_blob_sha1(&f).expect("hash");
    assert!(
        !test_sha_drift(&f, &recorded),
        "unchanged content must not read as drifted"
    );
    fs::write(&f, b"one: 2\n").expect("rewrite");
    assert!(
        test_sha_drift(&f, &recorded),
        "EDITED test content must read as drifted — otherwise a green survives a rewritten test, \
         which is exactly what happened on a gitignored spec suite"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn recorded_commit_is_a_full_sha1() {
    // `sha:` exists so someone can `git checkout <sha>` and re-run the test.
    // A 7-char short form is not durable evidence — it collides in a long-lived
    // repo and cannot be checked out unambiguously years later.
    if let Some(head) = git_head_sha1() {
        assert_eq!(head.len(), 40, "recorded commit must be the FULL sha-1, got {head:?}");
        assert!(
            head.chars().all(|c| c.is_ascii_hexdigit()),
            "recorded commit must be hex: {head:?}"
        );
    }
}
