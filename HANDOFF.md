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
and every intent covered. Track the count here after every subject round; the
number only moves through `arte verify`, never by hand.

- Round 0 baseline: 32 ok · 41 ko
- Round 1 (2026-08-04): 40 ok · 33 ko (+8, full-board verify through the
  live wall, ARTE_WALL_TIMEOUT_MS=120000). Taxonomy of 28 failing runs:
  17 real assertion fails, 9 timeouts, 2 harness (keyed-profile).
- Endgame (2026-08-05): first full gate FAILED on completeness (4 reds ·
  5 uncovered controls · 2 intent gaps · 99 orphan src files) — verify-green
  ≠ gate-green. Then: orphans 99→0 (103 honest stamps, 7 new impl nodes,
  3 new intents ADMITTED: team/feedback/QA-surface), 5 new controls + 5 new
  validations authored + PASS (team seat-cap / one-owner invariant /
  invite-claim / feedback / QA-boundary), events-attribution closed via
  get_events, one vacuous green rebuilt, flappers made order-independent.
  Incidents: an orphaned background gate corrupted results (→ run-lock intent
  on arte's board, serialization doctrine); both subagents hit session limits
  and the last mile went solo. Board now ~81 controls ok. FINAL gate running
  at handoff (PID-tracked, nohup + monitor — never pipe-background a gate).
- Rounds 5-9 (2026-08-04/05): 59 → 62 → 66 → 70 → 72 → **73 ok · 0 ko**.
  Everything that got there: selective verify (arte feature), fixture_reset as
  the enforced initial condition (verify setup hook), spec-lint gating the
  suite (parse/steps/js-compile — the wedge class died), console capture in
  run records, runner no-steps guard, mint_otp_link (mailbox-less magic-link),
  sign_stripe_webhook (real constructEvent boundary), cleanup_leads()
  extraction (PDPA rule verified for the first time), method:human relics
  rewritten machine-grade, one vacuous green caught and made real
  (history-groups). Full-gate run pending as the final judgment.
- **Round 2-3 (2026-08-04): 52 ok · 21 ko** (+12). Keyed-profile harness bug
  fixed (runner token fallback) + 13/13 disputed specs repaired and proven
  PASS individually. Remaining 21 ko: 3 deferred-by-ruling (fleet cells /
  dev-state update op / disposable user), ~9 undiagnosed impl reds (drafts,
  landing card, leads pagination, magic-link, realtime, scan-count, receipt
  handoff, delete-confirm), 3 Stripe-lane (checkout/portal/webhook — likely
  need Stripe test env), 3 unassessed (auth-user shop, PDPA retention,
  reminder template), ~3 FLAKY (draft-edits, theme-system, nav-tabs passed
  earlier same day — cross-test interference on the shared fixture; the
  stable-pass 2-of-5 window exists for exactly this).

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

## Subject round 1 (2026-08-04) — findings

- **Wall executor head-of-line starvation (FIXED in raanyang):** one queue entry
  with a test id unknown to the wall made `check()` silently return on the same
  first match every tick, starving all valid entries. `public/devwall/queue.js`
  now skips unknown-test entries (with a warn). Lesson for arte: silent `return`
  on a work-queue head is a whole-pipeline outage — skip + report, never hold.
- **Split-brain spec dirs (raanyang):** board `at:` → `tests/qa/` (all 69 stamps);
  wall API reads `qa/tests/`. 76/78 files byte-identical. Missing brand spec
  copied across; 2 stray unreferenced specs in `qa/tests` flagged, not deleted.
  Proper fix: ONE spec dir (route.ts QA path or board-wide re-stamp). Queued.
- Wall pipeline proven: executor claims + runs arte-queued tests; pass AND fail
  verdicts land in qa/runs and reach the lane runner.
- `ARTE_WALL_TIMEOUT_MS=120000` bounds each wall test — the interim form of
  friction intent #1 (arte-native `[verify] timeout` still queued).

## Subject round 2 (2026-08-04) — dispute rulings + spec repairs

16 disputed controls triaged: 13 specs repaired, 3 deferred with rulings
recorded on their controls (multiple-customers needs fleet per-origin cells;
checklist needs a dev-state update op; delete-account needs a disposable user —
never point it at qa1-4). Recurring spec-bug families — candidates for arte's
embedded test-author brief (bake-in queue):

1. **Stale references after remount/reload** — clicking a detached button is a
   silent no-op; every query must re-derive from a fresh document, and reload
   must wait for document identity to change. Hit 5 specs.
2. **Seed through the real path** — localStorage seeds vanish when the
   signed-in page's cloud read wins; seed the store the feature actually reads
   (here: handoff_sheets via dev-state, incl. `occurred_on` — the table
   defaults it to today and listSheets prefers it over created_at). Hit 4 specs.
3. **Falsifiable asserts only** — a seeded name containing the asserted pill
   word ("Overdue QA") made the assert vacuous; optional `if (el) fill(el)`
   guards silently skipped wrong-named fields for months. Hit 3 specs.
4. **Native dialogs ≠ React dialogs** — stubbing `window.confirm` does nothing
   to a component dialog; click its real `data-qa` confirm. Hit 4 specs.
5. **Readiness = the app's published signal, not a storage artifact** — an
   `sb-*-auth-token` key is per-origin and true on the OUTGOING document;
   raanyang publishes `.form-page[data-ry-shop="1"]` precisely for tests, and
   ignoring it left a ~700ms blind window where pushes/QR clicks are no-ops.
   Hit 3 specs (impl round 2 measurements).
6. **Initial condition is part of the contract** (user-articulated, the root
   of the whole flaky tail): a spec that declares preconditions in prose but
   runs against accumulated residue (qa1: 235 receipts / 301 leads / 86 scans)
   asserts nothing. Deterministic = enforced initial condition → controlled
   action → expected delta. Mechanism: raanyang's `[verify] setup` now runs
   dev-state `fixture_reset` before every verify; arte already had the hook.
7. **Observers die with their realm** — a fetch wrapper patched before
   navigation belongs to the discarded window and counts nothing; install on
   the destination window after load. Also: don't count rendered rows on a
   windowed list (slice(0,30)) — count the store. Hit 3 specs.

## Subject round 4 (2026-08-04) — the 9 reds resolved

Impl round: all 9 undiagnosed reds proved SPEC-side (measured disputes), zero
src changes. Test-author round: 9/10 specs repaired + probed PASS; the 10th
(new-lead realtime) was a real gap — `leads` absent from the supabase_realtime
publication (subscription SUBSCRIBED, zero events). Fixed via
`supabase/leads_realtime.sql` (replica identity full + idempotent publication
add), commit b621639. magic-link stays red: Supabase built-in SMTP quota
exhausted (429) — needs an SMTP provider or mailbox-less QA path, not code.

**Found, queued:** `stock_staging` is ALSO unpublished on dev (its migration's
publication line never landed — the publication held zero tables) → phone→PC
live stock list silently dead; one-line fix + check production. Fixture drift
worsens: probes/specs keep seeding qa1 (RT-PROBE-*, Pager-*, 200+ customers) —
a dev-state fixture-reset op is now the highest-leverage QA-infra item.

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
