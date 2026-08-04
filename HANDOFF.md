# Arte workflow handoff

## State at handoff (2026-08-04, round N+1)

**Board:** fully derived — `arte verify`: 19 ok · 0 ko · 0 holes; contracts + traces hold.
`arte gate` red with exactly **3 gaps** = the unbacked friction intents below.
Red gate = visible tool-round queue; backing an intent (control → validation → impl)
shrinks it. New-validation gotcha: `arte add validation` defaults subset TBD, which
falls to the DEFAULT lane (`npx vitest run`) — set `subset proof` (or your repo's
lane) right after minting, or verify runs the wrong command.

**Tests:** `cargo test --tests` green in ~9s (was >120s timeout).

## Fixed this round

1. **cli_surface 109s → 0.23s.** Root cause was NOT 23 slow spawns — 22 subcommands
   ran in <40ms each. `arte implement` bare fired the headless agent (`pi -p`
   default) + two gate passes. Fix: test spawns `implement` from a scratch cwd
   whose `arte.toml` sets `agent = "false"` (instant non-zero exit). Handler still
   reached, assertion intact, nothing commented out.
2. **Duplicate `prune_runs` consolidated** — the seq-based all-validations version
   is now the single `pub fn prune_runs(runs_dir, keep)` in `src/lib.rs`;
   the mtime per-id one is deleted. `write_run` + `verify_pass` both call it.
3. **`trace_pass` double-slash fixed** — claims are `trim_end_matches('/')`-normalized
   before the prefix test (`src/cli/verify.rs`).
4. **Vacuous test found + recorded** — `role_deny_dirs_covers_qa_tests_for_test_author`
   splits on the first `}` which lands past ALL match arms; its `.truth`/`qa/tests`
   matches come from the meta-planner arm. Worse: test-author's actual deny entries
   `.truth/impls`, `.truth/controls` deny nothing (`.truth/` is flat). DISPUTE note
   recorded on `c-role-isolation-table-...` — the SPECIFIER must rule (deny-list
   gains a real mechanism, or the control's claim shrinks). Do not "fix" the test
   before the ruling.

## NORTH STAR

**`arte gate` exits 0 on ../raanyang** — i.e. every raanyang validation `ok`
(currently 32 ok · 41 ko · 75 total, +2 uncovered intents) and every intent
covered. The 41 ko are real product work (Stripe checkout/portal, RLS,
realtime, offline sync…) — the loop grinds them down one subject round at a
time. Track the count in this file after every subject round; the number only
moves through `arte verify`, never by hand.

## THE LOOP: raanyang is the whetstone

Arte improves by grinding against a real subject. `../raanyang` (Next.js +
Supabase, 239-node board, 46 intents · 44 covered · 33 VERIFIED · 2 gaps) is
the subject repo. The loop alternates:

**SUBJECT ROUND (in ../raanyang):** run one normal arte round — observe, cycle,
dispatch trio per recipe, verify, gate. Goal: shrink its ko/gap count.

**Every friction hit during a subject round becomes an intent on ARTE's board**
(`arte add intent "…"` in this repo). A friction = a hang, a lie, a confusing
output, a refused commit, a lane violation the tooling didn't prevent.

**TOOL ROUND (in this repo):** back the oldest friction intent with
control → validation → impl, standard priority order (test > impl > spec).

That's the whole self-improvement mechanism. No new machinery — frictions are
just intents, the existing round discipline does the rest.

### Friction intents backed so far

- `i-subagent-lane-discipline-...` — **VERIFIED.** The commit protocol is now
  baked into the binary's embedded briefs (`role_lane` all four roles +
  AGENT_GUIDE subagent section, `src/lib.rs`), guarded by
  `tests/brief_protocol.rs`. Every `arte brief`/`guide`/`implement` in any
  repo now tells agents: workers never commit; orchestrator audits the
  footprint, then commits once per round with the intent id.
  **This is the standing rule: workflow lessons get baked into the embedded
  briefs (universal instruction), not just into this repo's recipes.**

### Friction intents currently seeded (unbacked — tool-round queue)

1. `i-lane-commands-cannot-hang-verify-forever-a-timeout-bounds-ever` —
   found live: raanyang `arte gate` hung >100s with zero output because its
   e2e/integration lane (`node scripts/arte-lane-runner.js`) polls a dev-wall
   executor that wasn't running. Needs `[verify] timeout = <secs>` in arte.toml,
   kill + `ko`(note: timeout) on expiry, and a heartbeat line so silence ≠ death.
2. `i-arte-implement-never-fires-the-headless-agent-as-a-silent-side` —
   bare `arte implement` launching an LLM agent is a landmine for any script or
   test that enumerates subcommands. Print what will run + require the agent to
   be configured explicitly (no `pi -p` silent default), or require a flag.
3. `i-run-timestamps-carry-the-real-date-the-epoch-to-civil-conversi` —
   run records stamp year **3996** (2026 + 1970): the epoch→civil conversion
   in `src/lib.rs` adds the 1970 base twice. One-line fix + a unit assertion.

## Known raanyang subject-round targets

- Many features `[ko] ⚡verified` — tests exist and are red. Classic impl-round work.
- 2 uncovered intents (`arte coverage` tail).
- `qa/queue.yaml<` — stray file (shell redirect typo?) in qa/. One-line cleanup.
- Its verify depends on a live dev-wall executor — until friction intent #1 is
  backed, run subject rounds with the executor UP, or the round wedges.

## Meta-planner discipline (unchanged)

- Don't write code. Dispatch, audit, hand off. Run `arte verify` / `arte gate`
  yourself to derive verdicts.
- Audit `git status --short -- .truth/ tests/ src/` after every hand-off.
- Status is derived, never hand-set — revert violations, tighten recipe, kill
  repeat offenders.
- One trio spawn per round; kill before respawn; duplicate spawns are pure waste.

## Files of interest

- `src/lib.rs` — canonical model + helpers (incl. the single `prune_runs`)
- `src/cli/{mutators,focus,query,verify,gate,cycle,lifecycle,role}.rs`
- `tests/` — 10 test files, all green, suite ~9s
- `recipes/claude-code/README.md` — discipline spec + NEW commit protocol
- `.truth/` — this repo's board (gate GREEN, 3 friction intents unbacked)
- `../raanyang/.truth/` — the subject board (239 nodes)
