# arte

**A design-truth board for AI-built software** — a git-native graph of
`intent → implementation → control → validation` that records *why code exists*
and *proves it still matches*, readable and writable by humans and any AI agent.

Think **"git for intent."** Git tracks what changed; arte tracks why it exists
and whether it's still true.

```
arte init          # scaffold the board in any project
arte guide         # the full protocol, printed for any first-contact agent
arte gate          # coverage + verify + contract + trace as ONE exit code (CI)
arte view          # live TUI visualiser (optional companion binary: arte-tui)
```

## The problem

AI agents drift. Ask one to build a feature and it writes plausible code with no
durable record of *what* was intended, *which rules* the code must obey, or *what
proves* it works. Docs-and-skills approaches (write a spec file, follow the
convention) all share one flaw: **they assume the agent complies.** A prompt rule
is not enforcement — an agent under task pressure games it, skips it, or
self-certifies.

arte's answer: don't ask the agent to behave — **make misbehavior fail the merge.**

## The model

Four layers, linked by `serves`, one file per node (`.truth/<id>.node`, plain
`key: value` lines — git's own line merge reconciles concurrent edits):

| layer | holds | example |
|---|---|---|
| **intent** | who / why / success | "Engineering students draw ISO 5807 flowcharts" |
| **impl** | what realizes it | "Canvas: drag-place, select, move" → `at: src/lib/diagram.js` |
| **control** | a CHECKABLE rule | "shapes snap: center to the 5 mm grid" |
| **validation** | a RUNNABLE test | `at: test/controls.test.js` — status **measured**, never asserted |

Everything else is **derived, never stored**: coverage, status roll-up, reverse
trace, orphans. See [FORMAT.md](FORMAT.md) for the complete format (it fits on a page).

## The enforcement ladder

1. **Measured status** — `arte verify` runs each validation's own test file and
   writes the result. A validation whose test is missing is a *hole*, never blessed.
2. **Contracts** — impls declare their public symbols; `arte contract` fails when
   the code stops exposing them (catches silent renames in regenerated code).
3. **Bidirectional trace** — `arte gate` fails on dangling `at:` pointers (board →
   missing file) *and* orphan src files no node claims (code the board doesn't own).
   An `at:` may claim a directory, so assets don't need bureaucracy.
4. **The gate** — `arte gate` = coverage + verify + contract + trace as one exit
   code. Install it as CI / pre-merge: enforcement moves from *write time* (agent
   compliance, gameable) to *accept time* (what the repo accepts).
5. **Separation of authority** — no agent grades its own work:

   | role | owns | may not touch |
   |---|---|---|
   | specifier | the board (intents, controls, decisions) | tests, code |
   | test-author | `test/` + validations (writes the RED first) | `src/`, the controls it scores |
   | implementer | `src/` (decides HOW, never WHAT) | tests, the board |
   | QA | milestone sign-off, dogfood → new controls | everything else |

   A **solo agent** runs the same roles as *phases*, same order, same boundaries:
   map first (read-only), pin behavior as tests, only then change code.
   `arte role <role> -- <cmd>` OS-sandboxes a role where arte spawns the process;
   `arte implement --watch` runs a headless implementer agent against the board.

## The loop (proven, not aspirational)

The reference project — a Vue flowchart editor for engineering students — was
built through **eight rounds** of this loop by three separated agents:

> specifier puts controls on the board → test-author turns them into failing
> tests (the torch) → implementer drives them green → orchestrator audits lanes
> and runs `arte gate` → repeat.

Outcome: 226 tests, 67 measured-green validations, 10/10 intents verified, and —
the part a skill can't give you — the loop **caught real defects in its own
spec**: the test-author *mathematically disproved* a routing rule (recorded as a
board amendment, not a silent rewrite); the implementer *disputed two test bugs
it was forbidden to edit* and was proven right by independent re-derivation; two
user-dogfood bugs that hid behind green unit tests became controls with
provenance and can no longer regress. Reproducibility held too: a fresh agent
rebuilt the app's behavior from the board alone.

## Quickstart

```sh
arte init                         # arte.toml + .truth/ + AGENTS.md (the protocol,
                                  #   auto-read by Claude Code/Codex/… on first contact)
arte guide                        # print the same protocol on demand
arte add intent "Who is this for" --subset who
arte add impl "The component" --serves <intent-id>
arte add control "on click X -> Y happens" --subset interaction --serves <impl-id>
arte add validation "X clicks do Y" --subset unit --serves <control-id>
arte at <validation-id> test/x.test.js     # stamp the proof
arte verify                       # run tests, DERIVE status
arte coverage                     # per-intent chain + holes
arte gate                         # everything as one exit code → CI
arte view                         # watch it live (arte-tui TUI, optional)
```

## Install

```sh
cargo install arte            # the board tool
cargo install arte-tui        # the optional viewer (`arte view` launches it)
```

Or from source (one clone builds the whole package):

```sh
git clone https://github.com/jakksaksrisuwan-tech/arte && cd arte
cargo build --release --workspace
cp target/release/arte target/release/arte-tui ~/.local/bin/
```

`arte` itself is a zero-dependency single file. The optional viewer (`arte-tui`,
a ratatui TUI with live refresh, pulse-on-focus, coverage/audit panels) lives in
[`viewer/`](viewer/) and is what `arte view` delegates to — skip installing it
and everything else still works.

## Design commitments

- **Identity is the id (filename), never the title** — renames can't break links.
- **One node per file, line-oriented** — git merges the board; no custom driver.
- **Layers are declared** (`arte.toml` chain), not hardcoded — domain-agnostic core.
- **Derive, never store** — anything computable from the graph is computed.
- **The tool judges structure; producers assert substance** — and `verify`
  measures the substance it can.
- **Zero dependencies.** The whole tool is one Rust file you can read in a sitting.
