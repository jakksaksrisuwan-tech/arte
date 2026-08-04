//! v-agent-guide-and-test-author-lane-state-the-enforced-initial-co
//! c-embedded-briefs-teach-the-initial-condition-doctrine-enforce-v
//!
//! Lesson from the raanyang rounds (user-articulated): the entire flaky tail
//! traced to specs whose preconditions lived in prose while the fixture
//! accumulated residue (235 receipts / 301 leads on one QA shop). One
//! harness-enforced reset turned three chronic flappers green in a single
//! run. The doctrine must reach every agent via the embedded briefs:
//! INITIAL CONDITION is part of the contract — enforce it (verify setup /
//! fixture reset), seed what you assert, never assume residue.

use arte::{role_lane, AGENT_GUIDE};

fn states_doctrine(text: &str) -> bool {
    let t = text.to_lowercase();
    t.contains("initial condition") && (t.contains("fixture") || t.contains("setup"))
}

#[test]
fn test_author_lane_states_initial_condition_doctrine() {
    assert!(
        states_doctrine(role_lane("test-author")),
        "role_lane(test-author) is missing the initial-condition doctrine — \
         a spawned test-author will keep writing asserts against accumulated residue"
    );
}

#[test]
fn agent_guide_states_initial_condition_doctrine() {
    assert!(
        states_doctrine(AGENT_GUIDE),
        "AGENT_GUIDE (the front door) is missing the initial-condition doctrine"
    );
}
