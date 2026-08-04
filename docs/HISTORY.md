# HISTORY — artefact's lineage

> **This is a historical document.** Written at artefact's birth; preserved
> here so the lineage lessons (still binding) don't disappear, but **not for
> first-contact orientation**. New users: read `README.md` first, then
> `FORMAT.md`, then `AGENTS.md`. For role boundaries + recipes, see
> `AGENTS_DUTIES.md` and `recipes/`.

---

# HANDOFF — arte

> **Historical document.** Written at arte's birth; kept for the lineage lessons
> (still binding). For what arte IS today — gate, verify, contract, roles, view —
> read `README.md` first, then `FORMAT.md`, then `src/main.rs`.

You're picking up a fresh primitive. Read this, then `FORMAT.md`, then `src/main.rs`.
It's small on purpose.

## What arte is (one line)
A **design-truth primitive**: a git-native graph of `intent → impl → control →
validation` nodes that records *why code exists* and *proves it still matches* —
readable and writable by humans and AI agents, mergeable along git branches.

Think "git for intent." Git tracks *what changed*; arte tracks *why it exists and
whether it's still true.*

## Lineage (don't relearn this the hard way)
arte is the **clean v2** of a prototype (`../ratatui_render`, crate `dtruth`) that
was dogfooded by building a real app on it, then reviewed by a principal-engineer
pass. Three lessons are baked into arte from line one — they are NOT up for redebate:

1. **Identity is a stable `id`, never the title.** The prototype keyed nodes on
   their visible title; a rename on one branch + an edit on another silently forked
   the node and orphaned every link to it. arte keys on an opaque id (the filename);
   `title` is a freely-renamable label. Links are by id.
2. **One node per file, line-oriented `key: value`.** So git's own line merge does
   per-field reconciliation — no big custom merge driver. The prototype stored the
   whole board as one JSON-ish file and needed a bespoke 3-way merge; arte mostly
   doesn't. (JSON was the wrong axis: optimize merge+review, not bytes.)
3. **Layers are declared roles (`arte.toml` chain), not hardcoded names.** The
   prototype hardcoded `intent/impl/control/validation` in five functions, so it
   wasn't really domain-agnostic. arte reads the chain.

## What's built
- `arte init` — scaffolds `arte.toml` (role chain) + `.truth/`, and seeds arte's
  own design truth (6 nodes — it dogfoods itself from commit 1).
- `arte observe` — reads the graph back, grouped by role in chain order.
- `FORMAT.md` — the node format spec.
- No dependencies. Plain text + git + a thin reader.

## What's next (build order — dependencies, not priorities)
1. **Mutators**: `arte add <role> "title" [--serves id]`, `arte set <id> <key> <val>`
   (status/at/note/sha). Each writes one `.node` file. Assign ids (short, opaque,
   collision-checked).
2. **Derive**: `arte coverage` (per spine node: served in every downstream role?
   roll-up status worst-wins) and `arte trace <id>` (walk `serves` up). Pure reads
   over `.truth/`. Orphans = a node whose `serves` id doesn't exist.
3. **Code edge** (the whole point): `arte verify` — scan source for `@trace <id>`
   comments, reconcile vs nodes (untraced / dangling / unrealized) across ALL
   downstream roles (the prototype only checked one — don't). Cross-check `at`
   locators actually exist in their file (cheap: substring; later: real parse).
4. **Merge**: rely on git per-file first. Add a tiny per-node field-merge driver
   ONLY for the same-node-edited-both-sides case (union `serves`/`at`/`note`;
   worst-wins `status`; conflict → annotate in `note`, never text markers).
5. **Clients**: a read-only viewer over the graph; then the live TUI (with the
   "what am I working on" pulse — it was the one genuinely novel thing); then MCP
   as a sibling transport (the read/mutate verbs ARE the MCP tools).
6. **Trust model** (when multi-agent): a **dev/QA two-party** split with
   **role-scoped writes** — dev writes intent/impl/control, only QA writes
   validation status. No self-applied gate makes a lying agent honest; independent
   verification + the audit trail is the real mechanism. An opt-in PreToolUse gate
   (generated per coding agent) is a forcing function, not security.

## Open decisions (your call, flag them)
- **id scheme.** Short opaque (`c1`, or a base32 stamp)? A human-visible stable
  ref (`ctrl_<area>_NNN`) can ride ALONGSIDE as a field for `@trace` anchors, but
  the **filename id stays the merge key**. Keep them separate; don't conflate.
- **note multiline.** Repeated `note:` lines (current) vs an indented block. Keep
  repeated lines unless a real case needs blocks — they merge better.
- **`at` verification depth.** Substring "ref present in file" first; a real
  per-language/-format parser only when that's not enough.

## Run it
```
cargo run -- init      # in a fresh dir (or this repo, already seeded)
cargo run -- observe
cat .truth/i1.node      # see the format
```

## The one rule
Build the **object model and its merge** before any UI. The predecessor built the
TUI first and the format paid for it. Object → derive → code-edge → clients.
