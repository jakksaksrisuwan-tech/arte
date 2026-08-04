# Recipes — concrete role separation patterns

> Quick rule: spawn roles as SEPARATE subagents. Specifier writes the
> board; test-author writes tests + validation nodes; implementer writes
> `src/`; QA dogfoods the running app. Don't fold them into one agent.
> See `../AGENTS_DUTIES.md` for the deep dive.

| harness | directory | notes |
|---|---|---|
| Claude Code (Agent tool) | `claude-code/` | Agent tool with `tools` filtered by directory |
| OpenAI Codex | `codex/` | worktree + sandbox flags |
| Hermes / subagent spawns | `hermes/` | `delegate_task` with role-scoped tool lists |
| Solo (no subagents) | `solo/` | phase separation in time |

## Common shape across all harnesses

Every role has the same outline — paste the right one into your harness:

```text
ROLE: <specifier | test-author | implementer | QA>
OWN: <directories this role may write>
MAY NOT: <directories this role may not touch>
TASK: <one-sentence brief>
ENTRY POINT: <first command to run — usually `arte observe`>
EXIT: <the artifact this role must leave behind>
DO NOT: <the things that would let this role grade its own work>
```

The harness-specific recipes below turn that outline into literal spawn
arguments.

## Trust model reminder

- **Auditing beats trusting.** Between hand-offs, check that the role
  stayed in its lane — `git diff <role-end-time>` should show only files
  under `<OWN>`.
- **Status writes go through `arte verify`.** Never have a role hand-set
  `status: ok`. If a validation is green, `arte verify` writes it; if it's
  red, the failure is real evidence in `qa/runs/<id>.yaml`.
- **Disputes escalate to the specifier.** Roles can talk; they cannot
  settle disputes between the two graded parties (that is collusion toward
  easy green).