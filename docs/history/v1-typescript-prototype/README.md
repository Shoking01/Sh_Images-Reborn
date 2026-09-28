# V1 design documents (TypeScript prototype, 2026-09-07)

**These documents describe an implementation that no longer exists.**

They are preserved here because they were the only remote copy, and because
the reasoning in them outlived the code. Everything here was written for a
Bun + TypeScript + `@gpuix/react` prototype. The application is now Rust +
GPUI, restructured into two crates with thirteen ADRs (see
[`docs/ARCHITECTURE.md`](../../ARCHITECTURE.md)).

## What is here

| File | Size | What it is |
| --- | --- | --- |
| `2026-09-07-v1-design-spec.md` | 12 KB | The approved V1 design: priorities, the frame-loop and RAM budgets, the module split, and the decisions taken during brainstorming. |
| `2026-09-07-v1-implementation-plan.md` | 122 KB | The task-by-task implementation plan (19 tasks across 5 slices) for the TypeScript prototype. |

## Why keep it

The prototype was abandoned in favour of a native Rust + GPUI implementation,
but not because the design was wrong. Two things survived the rewrite and are
worth reading for:

- **The domain decisions that had to be reinvented anyway.** Crop coordinate
  math, the export pipeline, the keymap model, and view geometry all appear in
  the plan and all had to be re-derived in `sh-core`. Where the two disagree,
  the Rust implementation is the truth — but the original is the record of what
  was tried first.
- **The budgets.** The performance and memory targets stated here are the same
  ones `AGENTS.md` still carries, which is not a coincidence.

## What is NOT here, and why

The prototype's source (`src/`, `tests/`), `package.json`, `bun.lock` and
`biome.json` were **not** preserved. They were dead code for a stack the project
left behind, and keeping them in a public repository would imply the project is
maintained.

The branch that held all of this, `feat/v1-initial`, was deleted after these
two documents were extracted.

## How to read these

As history, not as instructions. The plan opens with "For agentic workers:
REQUIRED SUB-SKILL…" — that framing applied to a build that was never
completed and does not apply here. Nothing in this directory is a build input,
a source of truth, or something to keep in sync.
