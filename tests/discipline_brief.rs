//! v-briefs-state-serialization-falsifiability-self-cleanup-and-the
//! c-embedded-briefs-teach-the-campaign-doctrine-serialize-derivati
//!
//! The raanyang campaign's distilled flows, verified into the briefs so any
//! agent in any repo inherits them from `arte guide` / `arte brief`:
//!   - derivations SERIALIZE (a concurrent verify corrupted a whole board;
//!     an orphaned background gate did it again),
//!   - specs are FALSIFIABLE and SELF-CLEANING (vacuous greens: counting
//!     calls on a shared global, names containing the asserted word,
//!     optional fills, truncated YAML, zero-test filters; leaked disposables
//!     displaced sibling specs),
//!   - verify-green is NOT gate-green (the gate judges completeness:
//!     coverage, per-control validations, orphan code, contracts).

use arte::{role_lane, AGENT_GUIDE};

#[test]
fn guide_states_serialization() {
    let t = AGENT_GUIDE.to_lowercase();
    assert!(
        t.contains("one verify") || t.contains("serialize"),
        "AGENT_GUIDE missing the serialization rule — concurrent derivations corrupt shared fixtures"
    );
}

#[test]
fn guide_states_gate_vs_verify() {
    let t = AGENT_GUIDE.to_lowercase();
    assert!(
        t.contains("verify-green is not gate-green") || (t.contains("gate") && t.contains("completeness")),
        "AGENT_GUIDE missing the verify-vs-gate distinction — agents will declare done at verify-green"
    );
}

#[test]
fn test_author_lane_states_falsifiability_and_cleanup() {
    let t = role_lane("test-author").to_lowercase();
    assert!(
        t.contains("vacuous"),
        "test-author lane missing the anti-vacuous-green laws"
    );
    assert!(
        t.contains("self-clean") || t.contains("clean up"),
        "test-author lane missing the self-cleanup rule — leaked fixtures displace sibling specs"
    );
}

#[test]
fn meta_planner_lane_states_serialization_and_final_gate() {
    let t = role_lane("meta-planner").to_lowercase();
    assert!(
        t.contains("serial") || t.contains("one verify"),
        "meta-planner lane missing derivation serialization"
    );
    assert!(
        t.contains("gate") && (t.contains("foreground") || t.contains("orphan")),
        "meta-planner lane missing the never-orphan-a-gate rule"
    );
}
