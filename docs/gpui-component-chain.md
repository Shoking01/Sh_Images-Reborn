# The gpui-component adoption chain

This branch is an **integration tracker** for a Feature Branch Chain. It is
never merged on its own; it exists so the eleven child pull requests have a
common parent and a single place to record how they fit together.

Review and merge the chain **bottom-up**. Each child PR's base is the previous
child's branch, so GitHub shows only that PR's own work unit. Merging is
strictly sequential — every link compiles only on top of its parent, so PR N
cannot merge before PR N−1.

## Why the work was split

The `spike/gpui-component` branch accumulated **4,555 authored lines** across
twelve work-unit commits, which is too much to review as one change. It is
split here into one deliverable per pull request, with each unit's tests and
documentation in the same PR as the behaviour they verify.

## The chain

| PR | Work unit | Lines |
| --- | --- | --- |
| 01 | Adopt `gpui-component` 0.7.0 on `gpui-pre` 0.3.7 for the top bar | 1,300 ⚠️ |
| 02 | Arm the thumbnail batch for the startup session | 142 |
| 03 | ADR-020, plus its measured binary-size correction | 154 |
| 04 | Recalibrate the size and memory budgets against measurement | 39 |
| 05 | Let a theme declare semantic slots | 588 ⚠️ |
| 06 | ADR-021 | 111 |
| 07 | Let a theme set its own hover strength | 243 |
| 08 | Read the theme's danger color for shortcut errors | 60 |
| 09 | Hold every grid cell to the preset row height | 124 |
| 10 | Assert grid and chrome layout as relations | 224 |
| 11 | Windows visual baselines for human review | 1,554 ⚠️ |

Line counts are additions plus deletions, excluding `Cargo.lock` and binary
assets.

## The three exceptions

Eight of eleven PRs land under the 400 changed-line review budget. Three do not,
and a single honest slicing pass could not bring them under:

- **01** — split four ways, the theme bridge is still 489 lines and the top-bar
  swap still 455.
- **05** — `theme.rs` alone is 415. It is one schema change and does not divide
  without breaking the change it describes.
- **11** — `capture.ps1` alone is 1,064 and is one cohesive tool.

These carry an explicit `size:exception`. No comment, test or document was
deleted or compressed to hit a number, because a smaller diff that says less is
worse than a larger one that says more.

## What was verified before the split

- The chain tip is **byte-identical** to the original `spike/gpui-component`
  branch: `git diff chain/11-visual-baselines spike/gpui-component` is empty.
  Nothing was lost or altered in the split.
- Every link passes `cargo check --workspace --all-targets`.
- The tip passes **549 tests**, `cargo clippy --workspace --all-targets
  --all-features -- -D warnings`, and `cargo fmt --check`.

## A note on the native review tooling

The native review candidate is derived from the merge-base against `main`, so
every branch in a chain resolves to the full accumulated diff rather than to
its own slice. Stacking does not change that: a stacked branch's merge-base
against `main` is still `main`. This chain is therefore built for human
review, and the size budget is enforced by the split rather than by tooling
that cannot see a slice.
