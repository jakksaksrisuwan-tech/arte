# Examples — what a healthy board looks like

> A picture is worth a thousand nodes. Below is a sketch of what a populated
> arte board looks like in practice — taken from real projects (the Vue
> flowchart editor referenced in the README, plus the tyre-shop SaaS in
> `jaakk/raanyang`).

## What a healthy intent looks like

```yaml
id: i-shop-ops-001
role: intent
subset: jobs            # from the chain's [subsets.intent] = ["who","jobs",...]
title: Shop owners issue service receipts from any device
note: |
  Who: shop owners (single-user shops) and their staff.
  Why: a paper receipt is a memory aid; a digital one is searchable,
       auditable, and survives phone-loss.
  Success: a new receipt entered on a phone at the shop is visible on
           the dashboard within 5 seconds, with no manual sync.
```

Good intents:
- name the WHO, the WHY, and the SUCCESS criterion in `note:`
- subset = `jobs` (or `who` / `why` / `features` / `constraints` / `success`)
- one intent per outcome the user cares about, not one per screen

## What a healthy impl looks like

```yaml
id: m-form-001
role: impl
subset: frontend       # from [subsets.impl] = ["frontend","backend","db"]
title: Public form view (cust pane)
serves: i-shop-ops-001
contract:
  - export default FormPage
  - export const DEFAULT_LOCALE
note: |
  /q/[token] renders the customer-facing form. Reads its draft from
  form_drafts via the service-role client; debounces writes 1.2s.
  Renders controlled inputs; submits to /api/intake.
at: src/app/q/[token]/page.tsx#FormPage
```

Good impls:
- declare the **public contract** as `contract:` lines — `arte contract`
  fails on rename, so a regeneration pass can't silently break consumers
- one impl per file/component, not one per screen-and-its-modal
- at: points to a real file#anchor the code actually exposes

## What a healthy control looks like

```yaml
id: c-form-001
role: control
subset: limit          # or ux / security / TBD
title: Save button on draft form is keyboard-activatable
serves: m-form-001
note: |
  keydown "S" with no modifier while focused inside .form-page calls
  the same save() handler the Save button click does. Field-focus is
  GUARDED: text input focused → S is literal text, not save.
  Spec'd because keyboard-only users and speed-repeat operators hit
  Save dozens of times per shift; mouse-only is the bottleneck.
```

Good controls:
- one criterion per control — "<event> on <target> → <observable change>"
- subset = `limit` (cap/edge) or `ux` (interaction) or `security`
- the criterion is **checkable** — not "looks right," but a measurable
  observable
- note explains WHY the criterion exists (the spec-derivation trail)

## What a healthy validation looks like

```yaml
id: v-form-001
role: validation
subset: proof          # from [subsets.validation] = ["proof"]
title: keydown "s" with no input focused calls save()
serves: c-form-001
at: qa/tests/form-save-keyboard-activates-save.yaml#step-2
status: ok             # WRITTEN by `arte verify`, never by hand
note: |
  Uses the devwall executor: focuses .form-page (not into an input),
  fires keydown s, asserts the save() call hit by spying on /api/intake.
  Setup provisions: rebuild=both panes, waitFor=.form-page.
```

Good validations:
- `at:` points to a **specific step** in the test file (test files have
  multiple steps; each step is one validation's evidence)
- status is **measured** by `arte verify` — `ok`/`ko`/`pending`/`justified`,
  written by the tool, not by humans
- note explains the **evidence** the test produces — what artifact a human
  reader sees when they open `qa/runs/<id>.yaml`

## What the chain looks like (intentional relationships)

```
i-shop-ops-001 (intent)
    │ serves
    ▼
m-form-001 (impl: frontend)
    │ serves
    ▼
c-form-001 (control: keyboard save)
    │ serves
    ▼
v-form-001 (validation: keydown "s" test)
    │
    └──► qa/tests/form-save-keyboard-activates-save.yaml
```

`arte trace i-shop-ops-001` walks the serves chain down; `arte trace
v-form-001` walks it up. Both directions should land at the same
validation file.

## What coverage looks like (per-intent summary)

```sh
$ arte coverage
i-shop-ops-001     verified:  7 / 8   (88%)   hole: v-form-005 — no test yet
i-shop-ops-002     verified:  4 / 4   (100%)  ✓
i-shop-ops-003     verified:  1 / 5   (20%)   holes: v-form-007..011 — control amended, tests missing
```

`verified` = the validation's `status: ok` is **measured** (from
`arte verify`), not just "covered" (a node exists). `hole` = a control or
validation exists but no test runs against it. **A green you can't defend
is worse than an honest gap.**

## What a dispute looks like (real example from the Vue project)

```yaml
id: c-routing-001
title: Connector arrows route along the shortest orthogonal path
note: |
  ORIGINAL: arrows route along the L1 path (Manhattan).
  DISPUTE (implementer): L∞ (Chebyshev) is shorter for diagonal moves and
    matches Figma's behaviour.
  SPECIFIER RULING: the test-author was right; L1 is the contract.
    A new control c-routing-002 was added for the optional L∞ mode
    (toggle in settings). See the board-history PR for the full
    derivation.
```

Disputes live as notes on the contested control. **The board is the court
record.** No "we agreed out-of-band" — every ruling is on the board.