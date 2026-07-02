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
