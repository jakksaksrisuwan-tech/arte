//! arte — a design-truth primitive.
//!
//! Truth is a graph of NODES. Each node is ONE file under `.truth/`, named by a
//! stable opaque id (`.truth/<id>.node`). A node is line-oriented `key: value`:
//! repeated keys = multi-value. Identity is the id (filename), NEVER the title —
//! so a rename is just a label change and links never break. One-node-per-file +
//! line-oriented means **git's own merge reconciles concurrent edits per field**;
//! the tool barely has to merge at all.
//!
//! CLI:
//!   arte init [--template software|arte]
//!                    scaffold `arte.toml` (default: software) + `.truth/`
//!   arte observe     read the graph back, grouped by role → subset
//!   arte add <role> "<title>" [--subset F --parent P --serves S]
//!   arte set <id> <key> <value>      arte unset <id> <key>       arte status <id> <v>
//!   arte link <id> <serves-id>       arte unlink <id> <serves-id>
//!   arte delete <id>
//!   arte coverage                    arte trace <id>
//!   arte runs <v-id>                 recent pass/fail window for one validation
//!   arte status <id> <v> [--requires-stable-pass] [--force]
//!                                    gate green claims on 2-of-5 stable-pass
//!   arte cycle --once|--forever      drive the PULL/CLASSIFY/DISPATCH/VERIFY/PROMOTE loop
//!   arte gate        coverage + verify + contract as ONE exit code (the CI merge gate)
//!
//! Read commands print a one-line staleness envelope (⏱  stale: verified at <sha>,
//! you're N commit(s) ahead) when a node's `sha:` predates HEAD — measured green
//! rolls past verification silently otherwise; this surfaces it without failing.
//!
//! Everything else (link, status, coverage, trace, reconcile, the git field-merge
//! driver, clients) is for the next agent — see HANDOFF.md.

use arte::cli;

fn main() {
    match std::env::args().nth(1).as_deref() {
        None => cli::query::observe(), // bare `arte` = front door
        Some("guide") | Some("help") | Some("-h") | Some("--help") => {
            print!("{}", arte::AGENT_GUIDE);
            std::process::exit(0);
        }
        Some("--version") | Some("-V") => println!("arte {}", env!("CARGO_PKG_VERSION")),
        Some("init") => cli::lifecycle::cmd_init(),
        Some("observe") => cli::query::observe(),
        Some("add") => cli::mutators::cmd_add(),
        Some("set") => cli::mutators::cmd_set(),
        Some("status") => cli::mutators::cmd_status(),
        Some("link") => cli::mutators::cmd_link(),
        Some("unlink") => cli::mutators::cmd_unlink(),
        Some("unset") => cli::mutators::cmd_unset(),
        Some("delete") | Some("rm") => cli::mutators::cmd_delete(),
        Some("working") | Some("focus") => cli::focus::cmd_working(),
        Some("at") => cli::mutators::cmd_at(),
        Some("check-commits") => cli::mutators::cmd_check_commits(),
        Some("coverage") => cli::query::cmd_coverage(),
        Some("trace") => cli::query::cmd_trace(),
        Some("show") => cli::query::cmd_show(),
        Some("runs") => cli::query::cmd_runs(),
        Some("cycle") => cli::cycle::cmd_cycle(),
        Some("verify") => cli::verify::cmd_verify(),
        Some("contract") => cli::verify::cmd_contract(),
        Some("gate") => cli::gate::cmd_gate(),
        Some("view") => cli::role::cmd_view(),
        Some("role") => cli::role::cmd_role(),
        Some("implement") => cli::role::cmd_implement(),
        Some("brief") => cli::lifecycle::cmd_brief(),
        _ => {
            eprintln!("usage: arte guide  |  arte init [--template NAME] | observe | add <role> \"title\" | set <id> <k> <v> | unset <id> <k> | status <id> <v> | link/unlink <id> <target> | delete <id>");
            std::process::exit(2);
        }
    }
}