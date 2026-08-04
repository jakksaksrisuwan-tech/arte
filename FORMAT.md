# arte node format — v0

Truth is a **graph of nodes**. Each node is **one file**: `.truth/<id>.node`.
A node is **line-oriented** `key: value`. That's the whole format.

```
id: c1
role: control
title: a node is keyed by a stable id, never its title
serves: i1
at: src/auth.rs#hash_password
status: ok
note: hashed with argon2id
```

## Rules

1. **One node per file**, named by its `id` (`.truth/<id>.node`). The id is the
   filename and the identity.
2. **`key: value`, one per line.** First `:` splits; the value is the rest of the
   line (so values may contain `:`). Leading `#` lines and blank lines are ignored.
3. **Repeated key = multi-value** (`serves:` twice = two links). Order preserved.
4. **Unknown keys round-trip untouched** — forward-compatible by construction.

## Fields

| key | card. | meaning |
|---|---|---|
| `id` | 1 | stable, opaque, assigned once, **never reused as a label**. Identity. |
| `role` | 1 | which layer; one of the chain in `arte.toml` (declared, not hardcoded) |
| `subset` | 0..1 | the role's **one grouping axis** (a column), value from its `[subsets]`. Its *meaning is named per layer* in `[subset_axis]`: intent → **framing** (who/jobs/…); impl → **component** (frontend/backend/db); control → **subset** (unnamed for now — limit/ux/security/TBD); validation → **proof**. One field, never per-layer fields. Default `TBD` where a layer expects a value but none is set (e.g. control). |
| `title` | 1 | human label — display only, **freely renamable** (it is NOT the key) |
| `note` | 0..n | how/what/why, one line each — design decisions live here; they **trickle down**: `arte show <id>` prints a node with every ancestor's notes (derived at read time) |
| `serves` | 0..n | UP-link, **by id** — the node(s) this realizes (toward intent) |
| `at` | 0..n | DOWN-link to the artifact unit: `file#unit` (`src/x.rs#fn`, `b.kicad#R1`). A `file:anchor` suffix (grep/vitest habit) is tolerated — everything after `#` or `:` is ignored when resolving the path |
| `status` | 0..1 | `ok` \| `ko` \| `pending` \| `justified`. On a test-backed validation it is **MEASURED** — `arte verify` runs the `at:` test and writes the result; hand-set green there is a lie the gate catches. Elsewhere it's asserted (`justified` = human-signed). |
| `contract` | 0..n | declared public symbol (one per line) the impl's source `at:` must expose; `arte contract` fails on a rename/drift |
| `sha` | 0..1 | commit the node was verified against |

**One classification axis.** `subset` is the grouping column; per-layer meaning
comes from `[subset_axis]`. (A reserved `category` second axis shipped in 0.1 and
was CUT in 0.2 — never used in practice; old boards' `category:` lines still
round-trip untouched, per the unknown-key rule.) The creep rule is **redundancy,
not count**: never add a field that re-classifies an existing axis. `modified`
is **derived** (file mtime, or git history) — never a stored field.

Identity = `id`. Links are by `id`. `title` is a label. (A predecessor prototype
keyed on title; one rename orphaned every link — the lesson is baked in here.)

## Why line-oriented, not JSON

The axis that matters is **merge + review**, not bytes. One node per file + one
field per line means **git's built-in line merge reconciles concurrent edits per
field for free** — branch A edits `status`, branch B edits `at` → clean auto-merge,
no driver. Repeated-key additions (`serves:`) merge cleanly too. JSON is a tree:
braces/commas create diff noise and need a custom per-field merge. Binary is
smallest but opaque/unmergeable/unreviewable — the wrong axis.

## Roles & subsets (`arte.toml`)

```toml
chain = ["intent", "impl", "control", "validation"]   # layer order

[subsets]                                              # the grouping values per role
intent     = ["who", "jobs", "why", "features", "constraints", "success"]
impl       = ["frontend", "backend", "db"]
control    = ["limit", "ux", "security", "TBD"]
validation = ["proof"]

[subset_axis]                                          # what `subset` is CALLED per role
intent     = "framing"
impl       = "component"
control    = "subset"
validation = "proof"
```

`chain` = the layer order coverage/trace walk. `[subsets]` = the grouping values per
role (**add/delete freely**; a node's `subset` must be one of them). `[subset_axis]` =
what the one `subset` field *means* in each layer — so impl subsets are "components",
validation subsets are "proofs", etc. (control's has no domain word yet — just
"subset"), without inventing per-layer fields. All declared,
so the tool keys semantics off `role`/`subset`, never hardcoded names — that's what
keeps it agnostic.

## Derived, never stored

Coverage, status roll-up (worst-wins: `ko > pending > justified > ok`), reverse
trace, and orphans are **computed** from the graph. The files hold only nodes and
their links.
