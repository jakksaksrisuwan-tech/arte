# viewer — arte-tui + arte-core

The optional TUI visualiser for arte boards (`arte view` delegates to the
`arte-tui` binary built here). Live refresh, focus pulse, coverage/audit panels.

- `core/` — presentation-free board model (validated wire protocol, state,
  agent digest, coverage/trace/audit). **May not depend on ratatui** — the
  boundary is compiler-enforced so other clients can link it.
- `tui/` — the terminal client (ratatui): render, control, mouse, theme.
- `examples/` — sample boards in the native JSONL wire format
  (`arte-tui <file>`); arte boards need no examples — `arte-tui arte <dir>`
  reads `.truth/` directly.

```sh
cargo install --path viewer/tui     # installs the `arte-tui` binary
arte-tui arte .                       # view the current project's board
```
