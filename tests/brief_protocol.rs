//! v-every-role-lane-text-and-the-agent-guide-state-the-commit-prot
//! c-embedded-role-briefs-carry-the-commit-protocol-workers-never-c
//!
//! Lesson learned running the workflow against a real subject repo: subagents
//! that commit their own work get refused at commit-audit time (lane
//! violations, missing intent ids) — after the tokens are spent. The fix is
//! universal instruction: every embedded brief (`role_lane`, AGENT_GUIDE)
//! must state the commit protocol, so ANY agent in ANY repo hears it at
//! spawn, not at refusal.
//!
//! FALSIFYING check: strip the protocol sentence from a lane text and the
//! matching assertion fails.

use arte::{role_lane, AGENT_GUIDE};

fn states_commit_protocol(text: &str) -> bool {
    let t = text.to_lowercase();
    t.contains("never commit") && t.contains("intent id")
}

#[test]
fn worker_lanes_state_the_commit_protocol() {
    for role in ["specifier", "test-author", "implementer"] {
        assert!(
            states_commit_protocol(role_lane(role)),
            "role_lane({role:?}) is missing the commit protocol (\"never commit\" + \"intent id\") — \
             a spawned {role} won't know its work must be committed by the orchestrator"
        );
    }
}

#[test]
fn meta_planner_lane_owns_the_commit() {
    let lane = role_lane("meta-planner").to_lowercase();
    assert!(
        lane.contains("never commit") && lane.contains("intent id"),
        "meta-planner lane must state that workers never commit and its commits carry the intent id"
    );
    assert!(
        lane.contains("audit"),
        "meta-planner lane must tie the commit to a passed lane audit"
    );
}

#[test]
fn agent_guide_states_the_commit_protocol() {
    assert!(
        states_commit_protocol(AGENT_GUIDE),
        "AGENT_GUIDE (the `arte guide` front door) is missing the commit protocol — \
         solo/orchestrator agents reading only the guide would never hear it"
    );
}
