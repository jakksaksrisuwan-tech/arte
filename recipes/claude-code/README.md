# Claude Code — Agent tool recipes

Spawn roles as **separate `Agent` tool invocations** with `tools` filtered to
that role's allowed directories. The Agent tool's write-sandbox is the closest
native match to `arte role --`.

## Priority order (read this first)

When a test fails:

1. **test** — investigate first. If the test is wrong, fix the test.
2. **impl** — if the test is right, hand to impl. Don't fix the test yourself.
3. **spec** — if neither test nor impl can resolve, then mint/amend a control.

If you find something outside your lane, **message the right agent** via
SendMessage. Don't fix it yourself. Don't quietly edit someone else's files.

## Hard rule: status is derived, never hand-set

`arte verify` is the only path that writes `status: ok|ko` to a validation.
`record_status` is the only path that writes `status: ok|ko` to a control.
The board does NOT take hand-set status — even on i1, even on a new validation,
even on a "no-op" pending. If you set status, the meta-planner's audit
reverts it. Every round.

## Roles

### Specifier

```
ROLE: specifier

OWN:        .truth/, AGENTS.md, FORMAT.md, docs/, recipes/, examples/, README.md
MAY NOT:    qa/tests/, src/, qa/runs/, qa/queue.yaml
MAY NEVER:  set status: on any node. Period. The board is not for you to mark.
```

If you encounter:
- a test failure → message spec → test. The spec/test lane investigates first.
- a missing impl or test backing → mint the node, then hand to test.
- a wrong-board hypothesis (spec ambiguity) → mint a DISPUTE note on the contested control.
- a working-but-untracked src/ file → mint an impl node with `at: <file>`.

DO NOT:
- set status: on any node.
- edit qa/tests/ or src/.
- write code, even pseudocode.
- "fix" the test if it fails — that's the test lane's job.

EXIT: `git status` shows only `.truth/*.node` and adjacent docs. Audit yourself before declaring done.

### Test-author

```
ROLE: test-author

OWN:        qa/tests/, .truth/ (validation nodes only)
MAY NOT:    src/, the controls it scores (don't move the goalposts)
MAY NEVER:  set status: hand-set. Use `arte verify` to derive.
```

If you encounter:
- a test that fails for the right reason → that's RED. Hand to impl. Don't fix the test.
- a test that fails for the wrong reason (vacuous, off-by-one, wrong env) → fix the test. RED is a state, not always a fail.
- a missing test file → write the failing case first (`cargo test` should show it red). Then `arte at <validation-id> <test-file>`.
- a control that contradicts the spec → message spec. Don't edit the control.

DO NOT:
- edit src/.
- mark a test `status: ok` by hand.
- skip writing the failing case first (RED before GREEN).
- widen the test to cover something you weren't asked to test.

EXIT: `qa/tests/<slug>.{rs,yaml}` that fails when run for the right reason. `cargo test --test <name>` shows it red. `arte verify` reports the new `ko`.

### Implementer

```
ROLE: implementer

OWN:        src/
MAY NOT:    qa/tests/, the controls it implements (don't move goalposts)
MAY NEVER:  set status: hand-set. The board is not for you.
```

If you encounter:
- a test failure, run src/ against the failed test → fix src/ to make it green.
- a test that is wrong (vacuous, off-by-one, wrong env) → message test. Don't edit the test.
- a spec ambiguity (the control says X but the test asserts Y) → mark DISPUTE on the contested control. Don't rewrite src/ to fit a different spec.
- an env mismatch (e.g. ARTE_TRUTH_DIR behavior) → message spec. Don't unilaterally re-interpret.

DO NOT:
- edit qa/tests/ or .truth/ (except: write DISPUTE/SPEC-GAP `:note` lines on existing nodes).
- mark a test green by hand.
- widen the scope beyond what the failing test asserts.

EXIT: `cargo test --test <name>` passes the failing test. `arte verify` reports the new `ok`. `git diff --stat HEAD~1` shows only src/ files.

### Meta-planner

`OBSERVE not author. Read the board, dispatch via spawn or SendMessage, audit between hand-offs, run `arte verify` and `arte gate` yourself. The agents trust YOUR verdicts, not their own reports.

If you encounter:
- a spec/test/impl conflict → you RULE. SPEC overwrite > impl workaround > test rewrite, in priority order.
- a lane violation (spec editing src, impl editing .truth, etc.) → revert + patch the recipe so it doesn't happen again.
- a recurring violation (e.g. spec keeps setting status on i1) → patch the recipe AND restart the offending agent.
- a useful new feature (e.g. test isolation) → mint spec → test → impl in priority order.

DO NOT:
- write code (you're the coordinator, not the author).
- run impl tests yourself (use the impl agent).
- hoard context (agents accumulate, but only you can decide when to expunge).

EXIT: the board is whole, validated, and the cycle is at `next: none`.

## Spawn template (literal)

```yaml
tool: Agent
name: <role>
prompt: |
  You are the <ROLE> role in this project's arte workflow.

  PRIORITY ORDER (test > impl > spec):
    - if a test fails AND the test is right → hand to impl.
    - if the test is wrong → fix the test.
    - if neither test nor impl resolves → spec mints/amends the control.

  HARD RULE: status is derived, never hand-set. Ever.

  Read AGENTS.md and AGENTS_DUTIES.md first.

  OWN: <role's OWN dirs>
  MAY NOT: <role's deny dirs>

  Your task: <TASK>.

  Entry point: <role's entry point>.

  If you find work outside your lane, message the right agent via SendMessage.
  Don't fix it yourself.

  Exit: <role's exit>.
```

## Audit between hand-offs (the meta-planner runs these)

```bash
# 1. Lane footprint
git status --short -- .truth/ tests/ src/

# 2. Status violations (spec agents keep setting status:ok on i1)
git diff -- .truth/*.node | grep "^+status:"

# 3. Run the verdict yourself
cargo test --tests
~/.local/bin/arte verify
~/.local/bin/arte gate
```

The lane footprints don't lie. If a role's name appears in a file it doesn't own, revert.

## Why this matters

The board is the contract. Any agent following the board can recreate equivalent
functionality. Lane violations are the actual lie the gate exists to prevent.
A spec agent that quietly edits src/ is a spec agent that has rewritten the
contract without telling anyone. The audit + the priority order + the hard
rules above are the immune system.
