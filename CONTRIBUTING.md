# Contributing to Sh Images

Sh_Images is a native, GPU-accelerated image viewer for Windows, written in
Rust with [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui).
The project has hard constraints that are not negotiable preferences: it is a
native Windows application with no web technology anywhere in the stack, and it
treats correctness, memory safety, and frame time as release blockers rather
than as polish.

Read [`AGENTS.md`](AGENTS.md) before your first change. It is the source of
truth for the engineering rules, and this document only summarizes what a new
contributor hits in practice. Where the two disagree, `AGENTS.md` wins.

## Prerequisites

- **Rust stable**, with `rustfmt` and `clippy`. The channel is pinned in
  `rust-toolchain.toml`, so `rustup` selects it automatically. No other Rust
  version is supported.
- **Windows** for anything release-related. The product targets Windows through
  DirectX, and every CI workflow runs on `windows-latest`. The
  `windows-latest` runner is the only environment where the quality gate and
  the packaging build are actually exercised. The pure logic in `sh-core` is
  portable, but nothing else in the tree is, and a change that only builds on
  Linux or macOS is not a supported contribution.
- **Inno Setup 6.3 or newer** only if you touch `installer/sh-images.iss`.

## Build and run

```sh
# Development build and launch
cargo run -p sh-app

# Open something specific
cargo run -p sh-app -- "C:\path\to\folder"
cargo run -p sh-app -- "C:\path\to\image.png"

# Release binary
cargo build --release -p sh-app
# -> target\release\sh-app.exe
```

## The quality gate

These three commands are the gate. All three must pass with zero warnings
before a commit. They are exactly what
[`.github/workflows/ci.yml`](.github/workflows/ci.yml) runs, so anything that
passes locally will pass there:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

`cargo fmt --all` (without `--check`) fixes formatting; do not hand-edit around
a formatting diff. Clippy is run with `--all-targets --all-features` and
warnings are errors, which means test code and every feature combination are
linted, not just the default build.

New code must ship with tests, and a public item must carry a `///` doc comment
that explains why, not just what. See `AGENTS.md` §4 for the per-module test
requirements and coverage thresholds.

## Constraints a contribution must not violate

These come from `AGENTS.md` §2, §3, and §7 and are enforced by review:

- **No JavaScript, no WASM, no web technology.** Not in the app, not in the
  build, not in the installer, not in a helper script. This is a native
  application; there is no DOM, no `document`, no `fetch`, no Node toolchain.
- **Core logic must not import GPUI.** `crates/sh-core` declares no `gpui`
  dependency at all and is `#![forbid(unsafe_code)]`. The two-crate split in
  [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) (ADR-001) is what enforces the
  layering — a `use gpui` in `sh-core` is a compile error, not a style
  violation. Presentation belongs in `crates/sh-app`.
- **No `unwrap()` or `expect()` in production paths.** Use `?`, `ok_or()`, or
  `match`. `unwrap()` is acceptable in tests and where an invariant is proven.
- **No `unsafe` without a `// SAFETY:` comment** explaining why it is sound.
  `sh-core` forbids it outright.
- **No `TODO` without an associated issue.** A `TODO` that references nothing
  is a rejected change.
- **No new dependency without the `AGENTS.md` §7.2 justification** recorded as
  a comment above the entry in `Cargo.toml`: why the standard library and the
  existing dependencies are insufficient, license, maintenance status, and
  compile-time impact.
- **No blocking I/O on the UI thread.** Decoding, scanning, and file dialogs go
  through `cx.spawn` or a background executor.
- **No `println!`.** Use `tracing` with an appropriate level.

## Commit convention

Conventional Commits, as used throughout this repository's history:

```text
<type>(<optional scope>): <imperative summary in the imperative mood>
```

Types in use here: `feat`, `fix`, `docs`, `ci`, `build`, `test`, `chore`, and
`refactor`. Scopes in use: `app`, `ui`, `viewer`, `settings`, `slideshow`,
`installer`, `ci`, `repo`, `release`. Examples from the log:

```text
fix(viewer): keep Back available with overlay
feat(ui): animate viewer controls
docs(release): record installer packaging and artifact-level verification
ci(release): publish installer artifacts with checksums and install smoke test
```

Keep the subject line under roughly 72 characters and describe what the change
does, not how the commit was produced. A `docs:` change adds documentation; a
`fix(installer):` change repairs the installer script. Anything that changes
compiled behavior or the dependency graph is not a `docs:` change.

## Changes to core logic need a second reviewer

`AGENTS.md` §5.3: **any change to `sh-core` requires review from another agent
or contributor** before merge, beyond the normal review. This is not
duplicated by the CI gate — passing `cargo test --workspace` is not a
substitute, because the concern is whether the logic is correct, not whether it
compiles.

A change to the architecture — the crate split, the settings schema contract,
the caching or rendering strategy, the packaging and update model — must also
update [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) with a new ADR in the
existing format: Context, Decision, Consequences, Alternatives considered.

## Installer gotchas you will hit immediately

`installer/sh-images.iss` has two failure modes that both look like something
else. Both are documented, with the evidence, in
[`docs/RELEASE_QA.md`](docs/RELEASE_QA.md).

**`installer/sh-images.iss` does not compile without a payload at
`target\release\sh-app.exe`.** Inno Setup resolves and compresses every
`[Files] Source:` at compile time, so a missing payload is a hard compile
error (`ISCC` exits 2 with `Source file ... does not exist`) even when you only
meant to check the script's syntax. This is expected, not a defect. Either
build the release binary first, or create an explicit stub at that path — which
is what the `installer-script` job in `ci.yml` does, writing a stub that is
never committed because `target/` is git-ignored.

**`[Icons] IconFilename` is a runtime path and is never validated.** Inno
writes the value verbatim into the `.lnk` and resolves it neither at compile
time nor at install time. A repository-relative value such as
`..\assets\branding\sh-images.ico` is accepted silently, and it resolves to
nothing at install time because the setup runs from a temp extraction
directory. The observable symptom is the worst kind: the install exits 0, the
setup log records "Successfully created the icon" with no warning, and the
shortcut points at a path that does not exist. Install the icon with `[Files]`
into `{app}` and reference `{app}\sh-images.ico` instead — and verify by
reading the created shortcut's real `IconLocation` back, never by trusting the
installer's exit code. The exact procedure is in `docs/RELEASE_QA.md`.

## Documentation changes do not need the gate

`ci.yml` sets `paths-ignore` for `**.md`, `LICENSE`, `NOTICE`, and `docs/**` on
both its `push` and `pull_request` triggers, because none of those paths can
change compiled behavior, the Cargo dependency graph, or the installer script.
**This means a documentation-only pull request produces no check runs at all**,
so review is the only gate it will ever get. Proof-read the diff: the claim
you state must be a claim you have actually verified against the code, the
workflow files, or a real workflow run.

A change to `installer/sh-images.iss` is *not* covered by that filter — it lives
under `installer/`, and the `installer-script` job compiles it on every pull
request.

## Reporting bugs and security issues

Non-sensitive defects go in the issue tracker. Vulnerabilities must not: follow
[`SECURITY.md`](SECURITY.md). Participation is governed by
[`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md).

## License

Contributions are accepted under the MIT License in [`LICENSE`](LICENSE).
Third-party attribution for what ships in the binary is in
[`NOTICE`](NOTICE); do not add third-party material without extending it.
