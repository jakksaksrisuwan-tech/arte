//! CLI command surface. Each submodule owns one concern.
//! `main.rs` matches on argv and calls into these.

pub mod mutators;
pub mod focus;
pub mod query;
pub mod verify;
pub mod gate;
pub mod cycle;
pub mod lifecycle;
pub mod role;