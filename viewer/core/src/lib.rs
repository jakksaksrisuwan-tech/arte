//! Design-truth core — the presentation-free substrate every client shares.
//! Holds the wire protocol, validation, state + mutation, the agent (observe/act),
//! and the sync transport. No rendering, no terminal — see the `tui` crate (or any
//! future web/native client) for that.
pub mod agent;
pub mod protocol;
pub mod state;
pub mod store;
pub mod sync;
pub mod validate;

/// How many chars of a commit sha to SHOW. `sha:` is recorded in full (40); this
/// is display only. It exists because the CLI truncated to 7 and the TUI to 8,
/// so the same board rendered a different width depending on which surface you
/// looked at. Mirrored by `arte::SHA_DISPLAY_LEN` — the root crate takes no
/// dependencies, so the constant is duplicated on purpose.
pub const SHA_DISPLAY_LEN: usize = 7;
