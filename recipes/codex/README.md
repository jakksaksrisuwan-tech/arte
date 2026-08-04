# OpenAI Codex — worktree + sandbox recipes

Codex uses git worktrees for isolation. The role separation maps cleanly:

- specifier gets a worktree of `main` with **read** access to all dirs and
  **write** access to `.truth/`, `*.md`
- test-author gets a worktree with write to `qa/tests/` + validation nodes
- implementer gets a worktree with write to `src/`

Codex's sandbox flags (when available) provide the closest equivalent to
`arte role --`. Combine with the `.gitignore` pattern to enforce.

## Pattern: worktree per role

```sh
# Specifier worktree (no src/ writes)
git worktree add /tmp/arte-spec HEAD
cd /tmp/arte-spec
# sandbox: --disable-edit src/, qa/tests/, qa/runs/
# prompt:
#   ROLE: specifier
#   OWN: .truth/, *.md
#   TASK: <TASK>
#   ENTRY: arte observe

# Test-author worktree
git worktree add /tmp/arte-test HEAD
cd /tmp/arte-test
# sandbox: --disable-edit src/, .truth/<control-impl>/
# prompt:
#   ROLE: test-author
#   OWN: qa/tests/, .truth/ (validation nodes only)
#   TASK: <TASK>
#   ENTRY: arte coverage

# Implementer worktree
git worktree add /tmp/arte-impl HEAD
cd /tmp/arte-impl
# sandbox: --disable-edit qa/tests/, qa/runs/, qa/queue.yaml
# prompt:
#   ROLE: implementer
#   OWN: src/
#   TASK: <TASK>
#   ENTRY: arte verify (read qa/runs/<id>.yaml for failure evidence)
```

## Pattern: branch + PR review

Each role produces a branch; the orchestrator reviews the diff between
branches and runs `arte gate` on the merged result:

```sh
git checkout -b spec/<topic>
# specifier commits .truth/* + *.md changes
git push origin spec/<topic>

git checkout -b test/<topic>
# test-author commits qa/tests/* changes (rebases onto spec/<topic>)
git push origin test/<topic>

git checkout -b impl/<topic>
# implementer commits src/* changes (rebases onto test/<topic>)
git push origin impl/<topic>

# orchestrator merges all three, runs arte gate, pushes to main
git checkout main
git merge --no-ff spec/<topic> test/<topic> impl/<topic>
arte gate  # coverage + verify + contract
```

## Codex-specific notes

- Codex's `--sandbox` flag isolates writes per session. Pass `--sandbox
  workspace-write` and a `.gitignore` to enforce role boundaries:

  ```text
  # .gitignore (each role's sandbox has a different version)
  # test-author sandbox:
  /src/

  # implementer sandbox:
  /qa/tests/
  /qa/runs/
  /qa/queue.yaml
  /truth/  # except DISPUTE/SPEC-GAP note: lines
  ```

- Codex CLI flags for isolation: `--disable-model-invocations`,
  `--sandbox=workspace-write`, plus the role-specific paths to ignore.
- For multi-step Codex runs (a single invocation that walks the loop),
  prefer **separate invocations** for separate roles — each invocation
  gets a clean sandbox + a fresh context.