# Solo mode — phase separation in time

When you don't have subagents (or don't want to spawn them), the four
roles become **phases**, same order, same boundaries. Sequencing in
time is what separation across agents degrades to.

## The phase contract

```text
PHASE 1: SPECIFIER (read-only)
  - read all the code
  - write .truth/*.node + adjacent *.md
  - exit: the board is current
  - do not touch src/ or qa/tests/

PHASE 2: TEST-AUTHOR (RED only)
  - for each unproven control, write a failing test in qa/tests/
  - stamp at: via `arte at <validation-id> <test-file>`
  - run `arte verify` — the new tests should FAIL
  - exit: red tests on disk, validation nodes with at: pointing to them
  - do not touch src/

PHASE 3: IMPLEMENTER (GREEN only)
  - read the red test evidence in qa/runs/<id>.yaml
  - change src/ to pass
  - if a test is wrong, write DISPUTE on the control (do not edit the test)
  - run `arte verify` — the new tests should PASS
  - exit: src/ changes, validation statuses flip ko→ok via verify
  - do not touch qa/tests/

PHASE 4: QA (dogfood)
  - run the actual app
  - click through milestone flows adversarially
  - find what unit tests miss
  - write SPEC-GAP notes for new defects
  - exit: milestone verdict or list of defects to address
  - do not implement fixes
```

## Why this discipline

The point of role separation is that **the goalposts and the scoring are
held by different parties**. If you're alone, you hold both — the test you
write, you might write to be easy. The discipline says: even alone, you
MUST treat the test as written by someone else, and the implementation as
a separate concern.

Concrete tricks:
- Write the test in one session. Commit it. Sleep on it. Then implement
  in a fresh session — pretend the test was written by a hostile party.
- If you find yourself "just adjusting the test to make it pass" — STOP.
  That's the goalposts moving. Write a DISPUTE note instead.
- If you find yourself "just adding a one-line guard" — STOP. That's
  scope creep. Update the control to declare the guard, then implement
  the guard.

## Minimal solo loop

```bash
# PHASE 1
arte observe
# write nodes for the new feature
git add .truth/ && git commit -m "spec: <feature>"

# PHASE 2 — write tests
git checkout -b test/<feature>
# write qa/tests/<slug>.yaml
arte at <validation-id> qa/tests/<slug>.yaml
arte verify   # should show RED for the new validations
git add qa/tests/ && git commit -m "test: <feature>"

# PHASE 3 — implement
git checkout -b impl/<feature>
# change src/
arte verify   # should show GREEN now
git add src/ && git commit -m "impl: <feature>"

# PHASE 4 — dogfood (you are the user now)
arte gate
# open the app, click through, look for SPEC-GAPs

# merge
git checkout main
git merge --no-ff test/<feature> impl/<feature>
arte gate
```

## When solo becomes painful

You outgrow solo when:
- The board has 50+ nodes and you lose track of which control you were
  pinning with which test
- The tests take >5 min to run; you can't remember what you were doing
- You start adjusting tests to pass rather than impl to match — a clear
  sign of the goalposts moving

When that happens: spawn the test-author and implementer as subagents
(see `../claude-code/` or `../hermes/`). The roles are designed to be
distributed.

## Sanity check before declaring "done"

- [ ] `arte verify` exits 0
- [ ] `arte gate` exits 0 (coverage + verify + contract)
- [ ] `arte contract` reports no drift
- [ ] The node I was on (`arte working <id>`) has `status: ok` derived
- [ ] The build/test command from `arte.toml [verify]` runs green from a
      clean checkout
- [ ] I haven't edited any test, control, or status field since PHASE 2