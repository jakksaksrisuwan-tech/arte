# Roles & recipes — how the four lanes work

This is the deep dive on role separation. `AGENTS.md` is the first-contact
digest; this file is what you read once you know *what* to do and need
*how*.

## The four roles

| role | owns | may not touch |
|---|---|---|
| specifier | the board (intents, impls, controls, decisions) | `src/`, tests, validation `status` |
| test-author | `qa/tests/` + validation nodes (writes the RED first) | `src/`, the controls it scores, hand-set `status: ok` |
| implementer | `src/` (decides HOW, never WHAT) | tests, the board, validation `status` |
| QA | milestone sign-off, dogfoods the running app for what unit tests miss | implementation, the board |

**The point**: whoever sets the goalposts is never who scores against them.
That — not a gate you can perform — is what makes the workflow hard to game.

Enforce it, don't trust it:

- `arte role <role> -- <command>` runs a command under a role's write-isolation
  (implementer can't touch tests or `.truth/`; test-author can't touch `src/`).
- `arte implement --watch` runs a headless implementer agent against the
  board with this isolation baked in.
- **For subagent harnesses** (Claude Code Agent tool, Codex, Hermes, etc.) —
  spawn the roles as **separate subagents** with their `tools` filtered by
  directory. See `recipes/` for harness-specific prompts.

## Subagent recipes (TL;DR)

The detailed prompts are in `recipes/<harness>/<role>.md`. Quick rule of thumb:

- **Spawn roles as SEPARATE subagents.** You stay ORCHESTRATOR + specifier.
- **Test-author gets `qa/tests/` + `.truth/` write access.** It turns each
  unproven control into a FAILING test and stamps `at:` via `arte at`. A
  not-yet-built feature SHOULD be red — that red is the implementer's work
  order (the torch).
- **Implementer gets `src/` write access only.** Its whole spec is the
  board + the red tests; it stamps `arte at` on impls it realizes, nothing more.
- **Audit between hand-offs.** Don't trust agent reports — check each lane's
  footprint (tester touched no `src/`; implementer touched no `test/` or
  statuses), run `arte verify` yourself to derive status.
- **Cross-talk is allowed.** The walls bound writes, not speech. Roles may ask
  each other questions directly.

## Disputes & spec-gaps (the feedback loop)

- **DISPUTE** (implementer disputes a test): implementer writes `DISPUTE: <id>
  — <reason>` as a note on the contested control. Specifier verifies
  independently and rules:
  - test wrong → test-author fixes per the ruling
  - impl wrong → dispute rejected
  - control ambiguous → specifier amends the control FIRST, then the fix cascades
  - **rulings live as notes on the control** — the board is the court record

- **SPEC-GAP** (tester or implementer finds the spec MISSED something): they
  write `SPEC-GAP: <missing thing>` as a note. Specifier triages: mint a
  control, amend one, or decline with the existing home named. Proposals from
  anyone; minting only by specifier.

A spec improved from downstream is the loop working. Nobody ever edits the
artifact that grades them.

## Be detailed — where boards fail

- **Decompose by behaviour.** For software, one control per interaction:
  `<event> on <target> → <observable state change>`. Cover pointer, drag,
  wheel AND keyboard (`keydown` on the document, **guarded when a text field
  is focused**). E.g. `pointerdown on empty + move → pan`; `keydown Delete →
  remove selection`.
- **Declare public contract** as `contract:` lines on the impl (one
  export/signature each). `arte contract` checks the code exposes them, so a
  rename is caught.
- **Interactions between features.** Enumerate every pair (operator ×
  operator, mode × edge, state × event) and give the pair's trickiest corner
  its own control. Property/round-trip tests must include adversarial corners
  BY CONSTRUCTION (e.g. unary minus WITH exponentiation), not only random draws.
- **Styling is measurable.** Colour saturation, px sizes, contrast ratios —
  not "looks right."
- **Order by validation difficulty.** Spec and green the HARDEST-to-validate
  lanes first (interaction, e2e, reachability). Backend-first green is a
  measured trap: fast hollow green with all the risk deferred to the end.
- **Every UI surface needs a REACHABILITY control** — a user can navigate to
  it from the entry screen. Unit-green orphan screens are a measured
  failure mode. Then milestone `qa` validations run the REAL app
  adversarially — unit tests share the build's blind spots; only the running
  artifact exposes what nobody specced.
- A control with no checkable criterion, or no validation, is a
  reproduction hole.

## Solo mode (no subagents)

The roles become **phases**, same order, same boundaries:

1. Act as **specifier** — map the existing code into intents/impls/controls.
   Read-only.
2. Act as **test-author** — pin current behaviour as validations BEFORE
   changing anything. **RED first.**
3. Act as **implementer** — change `src/` against those pins. **GREEN next.**

Never skip a phase because you're alone. Sequencing in time is what
separation across agents degrades to.

## Board hygiene (churn kills signal — field-measured)

- **STAMP, don't re-mint.** An existing stub is your home — `arte at` it.
  New nodes only where a real gap has no home (`arte coverage` shows holes).
- **A finding attaches to the control that governs its surface.** New
  control only for ungoverned surface. No reflexive
  one-node-per-finding fan-out.
- **Descope by deleting the subtree** (`arte delete`). The board is current
  scope, not a wishlist; dead nodes read as work to every agent that
  comes after.

## Sanity checks before declaring "done"

- `arte verify` exits 0
- `arte gate` exits 0 (coverage + verify + contract)
- `arte contract` reports no drift
- The node you're working on (`arte working <id>`) has its `status: ok` —
  not hand-set; derived from `arte verify`
- The build/test command from the project's `arte.toml [verify]` block runs
  green from a clean checkout