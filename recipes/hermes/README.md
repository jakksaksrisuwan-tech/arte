# Hermes — `delegate_task` recipes

Hermes dispatches subagents via the `delegate_task` tool. The role's
isolation is enforced by the prompt itself + a sandbox check on tool
allow/deny at the parent level.

Hermes also has an **orchestrator discipline** built into its skill system
(meta-planner). The orchestrator must NEVER edit the artifact files of the
roles it dispatches — only read, classify, dispatch, and verify.

## Specifier

```python
delegate_task(
    goal="""You are the SPECIFIER role in this project's arte workflow.

Read AGENTS.md and AGENTS_DUTIES.md first.

Scope: ONLY write to .truth/, AGENTS.md, FORMAT.md, docs/, recipes/.
MUST NOT touch qa/tests/, src/, qa/runs/, qa/queue.yaml.

Task: <TASK>.

Entry point: `arte observe`.
Do NOT set status fields on any node — `arte verify` derives those.
Exit: a clean `git status` showing only .truth/*.node and adjacent docs.
""",
    context="Project: <repo>. arte board at .truth/. <any other relevant context>",
    role="leaf",
)
# Suggested tool allowlist at the orchestrator level (Hermes filters by tool name):
# - read_file, write_file, patch (for .truth/*, *.md)
# - search_files, terminal (for `arte ...`)
# - NOT patch to qa/tests/, src/, qa/runs/, qa/queue.yaml
```

## Test-author

```python
delegate_task(
    goal="""You are the TEST-AUTHOR role in this project's arte workflow.

Read AGENTS.md and AGENTS_DUTIES.md first.

Scope: ONLY write to qa/tests/ and .truth/ (validation nodes only).
MUST NOT touch src/ or amend the controls you score.

Task: <TASK>.

Entry point: `arte coverage`. Look for `pending` / `holes`.
RED BEFORE GREEN — write the failing case first. Use `arte at` to stamp.
Exit: tests that fail when run; `arte verify` shows the new reds.
""",
    context="...",
    role="leaf",
)
# Tool allowlist:
# - read_file, write_file, patch (qa/tests/* and .truth/* validation nodes only)
# - search_files, terminal (`arte verify`, lane runner)
# - NOT patch to src/ or any control/impl node
```

## Implementer

```python
delegate_task(
    goal="""You are the IMPLEMENTER role in this project's arte workflow.

Read AGENTS.md and AGENTS_DUTIES.md first.

Scope: ONLY write to src/.
MUST NOT touch qa/tests/ or .truth/ (except writing DISPUTE/SPEC-GAP notes).

Task: <TASK>.

Entry point: `arte verify`. Read qa/runs/<id>.yaml for the actual failure
evidence. Don't trust agent reports — read the run file.
Stay within the contract declared on each control.
Don't move goalposts — if a test is wrong, raise DISPUTE.
Exit: src/ changes that turn the reds green. `arte verify` re-derives status.
""",
    context="...",
    role="leaf",
)
# Tool allowlist:
# - read_file, write_file, patch (src/* only)
# - search_files, terminal (`arte verify`, lane runner)
# - NOT patch to qa/tests/, qa/runs/, qa/queue.yaml, or any .truth/ node
```

## QA

```python
delegate_task(
    goal="""You are the QA role in this project's arte workflow.

Read AGENTS.md and AGENTS_DUTIES.md first.

Scope: READ-ONLY across the repo. You may write milestone notes on
.truth/ nodes only.

Task: dogfood the running app for the milestone flows. Find what unit
tests miss.

Entry point: `arte view`. Open the app at the documented entry screen.
Exit: either accept (write `note: milestone-<date> QA: <verdict>` on the
relevant nodes) or list SPEC-GAP notes for each defect.
""",
    context="...",
    role="leaf",
)
# Tool allowlist:
# - read_file (all paths)
# - terminal (`arte ...`, dev server)
# - WebFetch, WebSearch (running app)
# - search_files
# - write_file ONLY for milestone notes on .truth/ nodes
```

## Orchestrator discipline (CRITICAL for Hermes)

Hermes's meta-planner skill explicitly forbids the orchestrator from
editing `qa/tests/`, `src/`, or `.truth/<v-id>.node`. The Plasmid cycle is:

```
PULL    read qa/runs/, .loop-dispatch.json
CLASSIFY  run scripts/attr.py (or the spec-department skill) to bucket each failure
DISPATCH  delegate_task to the right dept skill with explicit role + scope
VERIFY    run scripts/loop.sh --once and let `arte verify` derive statuses
PROMOTE   only on stable-pass (2 of 5 recent runs), write `status: ok` via `arte status <id> ok`
```

Direct edits by the orchestrator break role separation and the
stable-pass rule. The skill names it as the #1 cause of "inflated counts"
in real-world use.

## Useful Hermes patterns

### Audit between hand-offs

```python
# After test-author returns:
subprocess.run(["git", "diff", "--stat", "HEAD~1"], cwd="<repo>")
# Should show only qa/tests/*.yaml and validation nodes.
# If src/* or control nodes are touched — role boundary violated.
```

### The loop script handles PULL + CLASSIFY + PROPOSE

```bash
bash scripts/loop.sh --once    # single iteration
bash scripts/loop.sh --forever # daemon mode
```

`loop.sh` writes `.loop-dispatch.json` and `.loop-history.json`. Read these
to know which test → which bucket → which dept.

### Stable-pass rule

A test is "stable-pass" when 2 or more of its last 5 runs PASS. Only then
does the orchestrator write `status: ok` to the validation. Otherwise:

- latest pass but < 2/5 → leave status as-is
- latest fail → write `status: ko`
- 3 dispatch rounds on same bucket → **quarantine** (write a quarantine note
  in `.truth/<id>.node` rather than delete; the user reviews + deletes)

### Where to put quarantine notes

If a test fails repeatedly and the loop's classification keeps landing on
the same bucket for 3 rounds, the orchestrator should write:

```yaml
note: "WHY: <root cause>. WHEN: <conditions to fix>. EFFORT: <S/M/L>"
```

to `.truth/<v-slug>.node`. This is the meta-planner's escape hatch — the
human can triage later from these notes.