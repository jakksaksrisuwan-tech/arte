//! Helper test file for the run-history pruning test in `test_isolation.rs`.
//! Used as the `at:` target of a synthetic validation that is seeded into
//! a scratch truth dir for the prune test. We deliberately keep this file
//! minimal and free of any sub-spawn of `arte` so the verify run that
//! grades this validation does NOT recursively call back into the test
//! harness — avoiding a ratchet where each recursion adds more run files.

#[test]
fn noop_target_for_prune_test() {
    // Intentionally empty — its job is to be a passing test target so
    // `arte verify` has something to grade during the prune test.
}
