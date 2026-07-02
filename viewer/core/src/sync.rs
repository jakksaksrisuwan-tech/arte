//! Live-sync transport: the glue that lets a running TUI and an external agent
//! share one session through sidecar files. All the inbox/state/lock plumbing
//! lives here so `main` just opens a channel and pumps it.
//!
//! TUI side: `Channel` (owns the read offset). Agent CLI side: the free fns.

use crate::agent;
use crate::state::AppState;
use crate::store;

/// The TUI's end of the live channel.
pub struct Channel {
    inbox: String,
    state: String,
    lock: String,
}

impl Channel {
    /// Mark a TUI live and start with a clean inbox (no stale-command replay).
    pub fn open(store_path: &str) -> Self {
        let c = Channel {
            inbox: store::inbox_path(store_path),
            state: store::state_path(store_path),
            lock: store::lock_path(store_path),
        };
        let _ = std::fs::write(&c.lock, "live");
        let _ = std::fs::write(&c.inbox, "");
        c
    }

    /// Apply any new agent commands from the inbox to live state. Returns true
    /// if anything was applied. Rename-swaps the inbox first: the atomic rename
    /// claims the current batch, so a command appended mid-drain lands in a fresh
    /// inbox and is picked up next tick — no read-then-truncate lost-tail window.
    pub fn drain(&mut self, app: &mut AppState) -> bool {
        let swap = format!("{}.swap", self.inbox);
        if std::fs::rename(&self.inbox, &swap).is_err() {
            return false; // nothing to drain (no inbox yet)
        }
        let txt = std::fs::read_to_string(&swap).unwrap_or_default();
        let _ = std::fs::remove_file(&swap);
        let mut changed = false;
        for line in txt.lines() {
            let _ = agent::dispatch(app, line); // ignore bad commands
            changed = true;
        }
        changed
    }

    /// Publish the current digest for the agent to read.
    pub fn publish(&self, app: &AppState) {
        let _ = std::fs::write(&self.state, agent::observe(app, None));
    }

    /// Release the live marker; the agent falls back to direct file ops.
    pub fn close(&self) {
        let _ = std::fs::remove_file(&self.lock);
    }
}

// --- agent CLI side ----------------------------------------------------------
pub fn is_live(store_path: &str) -> bool {
    std::path::Path::new(&store::lock_path(store_path)).exists()
}

pub fn send(store_path: &str, line: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(store::inbox_path(store_path))?;
    writeln!(f, "{line}")
}

pub fn read_state(store_path: &str) -> Option<String> {
    std::fs::read_to_string(store::state_path(store_path)).ok()
}
