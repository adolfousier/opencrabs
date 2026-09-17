# Contributing to OpenCrabs

Thank you for your interest in contributing to OpenCrabs! We welcome contributions from the community — but we have clear standards to keep the project moving forward efficiently.

## Before You Do Anything

**Read this entire document.** PRs that ignore these guidelines will be closed without review.

## Types of Contributions

### 1. Bug Reports (Issues Only)

Found a bug? Open an issue — **do not submit a PR yet**.

**Required information:**
- Clear, descriptive title
- Steps to reproduce (exact commands, config, inputs)
- Expected vs. actual behavior
- Environment: OS, Rust version (`rustc --version`), OpenCrabs version (`opencrabs --version`)
- Full error messages and logs (redact API keys)
- Screenshots if it's a TUI/visual issue

### 2. Feature Requests (Issues Only — No Code)

Have an idea for a new feature? Open an issue with the `enhancement` label.

**What to include:**
- What problem does this solve?
- How should it work from the user's perspective?
- Why is this useful to OpenCrabs users broadly (not just your use case)?

**What NOT to do:**
- Do not submit a PR with stub/placeholder code for a feature that doesn't exist yet
- Do not submit empty implementations with `todo!()`, `vec![]`, or `unimplemented!()`
- Do not submit PRs that add files with no actual logic, no tests, and no integration

**Stub PRs will be closed immediately.** If you want a feature built but don't have the skills to implement it, that's totally fine — open an issue, describe what you need, and the community or maintainers will pick it up. A well-written issue is 10x more valuable than a stub PR.

### 3. Code Contributions (PRs)

PRs are welcome for:
- Bug fixes (reference the issue number)
- Feature implementations (must have a linked issue approved by a maintainer first)
- Performance improvements (with benchmarks showing the improvement)
- Test coverage improvements
- Documentation fixes

## Issues Must Be Atomic

**One issue = one atomic piece of work.** A feature can have dozens of issues — each tracking one bug, one sub-task, or one improvement. Don't bundle multiple pieces of work into a single issue.

**Why:** Atomic issues are easier to track, prioritize, and close independently. A single "mega-issue" covering 5 bugs becomes a blocker when only 3 are fixed.

**Good:**
- `#376 — split_message byte boundary`
- `#377 — Rich API retry`
- `#378 — strip_html_tags incomplete`

**Bad:**
- `#375 — Fix all plain-text fallback bugs` (covers 3 separate issues)

**PRs can reference multiple atomic issues:**

```markdown
## Summary
Fixes message delivery cascade that caused plain-text fallback.

Fixes #376 (byte/char boundary), #377 (rich API retry), #378 (HTML tag stripping).

## Changes
- `split_message()` now uses `chars().count()` instead of `.len()`
- Added `RetryAfter` handling to rich path
- Completed `strip_html_tags()` to handle all HTML elements
```

This lets you batch related fixes in one PR while keeping the issue tracker clean and atomic.

### Issue Titles

Issue titles use the same [Conventional Commits](https://www.conventionalcommits.org/) shape as commit messages: `<type>(<scope>): <what is wrong or wanted>`. The type is one of `fix`, `feat`, `docs`, `refactor`, `test`, `chore`, `ci`; the scope is the module or surface (`tui`, `provider`, `rsi`, `telegram`, `memory`).

```
fix(tui): copy-to-clipboard notice shifts the chat history three rows
feat(provider): send tool_stream to z.ai so tool-call arguments stream
docs: README test counts are stale
refactor(memory): mod.rs is declarations-only
```

A bare area prefix (`TUI:`, `z.ai:`, `Reasoning stream:`) is not the convention. The type makes the tracker filterable and lets the fixing commit reuse the title verbatim. Add labels on creation (`--label bug --label tui`), one for the type and one for the area.

## Step-by-Step: Submitting a Bug Fix

1. **Find or create the issue** — Check existing issues first. If none exists, create one.
2. **Wait for confirmation** — A maintainer will confirm it's a real bug and not a duplicate.
3. **Fork and branch** — Fork the repo, create a branch from `main` (not `master`).
4. **Fix the bug** — Keep changes minimal. Fix the bug, nothing more.
5. **Add a test** — Write a test that fails without your fix and passes with it.
6. **Run CI checks locally** (see below).
7. **Submit the PR** — Reference the issue, explain what you changed and why.

## Step-by-Step: Submitting a Feature

1. **Open an issue first** — Describe the feature, get maintainer approval.
2. **Discuss the design** — For non-trivial features, discuss the approach in the issue before writing code.
3. **Fork and branch** from `main`.
4. **Implement fully** — The feature must work end-to-end. No stubs, no placeholders, no "TODO: implement later".
5. **Add tests** — Unit tests at minimum, integration tests for complex features.
6. **Run CI checks locally** (see below).
7. **Submit the PR** — Reference the issue, include before/after screenshots for UI changes.

## Development Setup

### Prerequisites

- **Rust** 1.91 or later (edition 2024)
- **SQLite** (bundled via `rusqlite`)
- **Git**

### Build & Test

```bash
# Clone your fork
git clone https://github.com/YOUR_USERNAME/opencrabs.git
cd opencrabs

# Iterate locally
cargo clippy --all-features         # USE THIS — never `cargo check` or `cargo build`
cargo test --all-features            # run the suite (incl. your new tests)
cargo fmt --all                      # auto-format before committing

# Run the EXACT CI checks (you MUST pass all three before submitting a PR)
cargo fmt --all -- --check
cargo clippy --lib --bins --tests --examples --all-features -- -D warnings
cargo test --all-features --verbose
```

**All three commands must pass.** PRs with failing CI will not be merged. We'll comment on the PR explaining what's failing and how to fix it. Push the fix, wait for CI to go green, and the PR will be reviewed.

`cargo clippy` is the lint pass we trust — `cargo check` only type-checks and misses the lint rules CI enforces. Iterate with clippy locally so you don't burn a CI run discovering a `-D warnings` failure.

#### Why clippy, not `cargo check`

Most Rust users learned `check` before they met clippy, so the habit dies hard. The distinction is not style policing: clippy runs rustc itself (so it reports everything `cargo check` does) plus ~700 lints, and the [`clippy::correctness` group](https://doc.rust-lang.org/clippy/lints.html) is deny-by-default and aborts compilation: "if you see a correctness lint, it means that your code is outright wrong or useless". Those lints are deliberately low-FP, and `#[allow]`-ing them is discouraged. A clean `cargo check` still passes code like:

- `let _ = parking_lot_mutex.lock();` - the guard drops on the next line, so the "locked" section protects nothing. `clippy::let_underscore_lock` (correctness) aborts; `cargo check` is silent.
- `#[derive(PartialOrd)]` alongside a hand-written `impl Ord` that disagrees with it. Sorting and `BTreeMap` keys then violate their ordering invariants at runtime. `clippy::derive_ord_xor_partial_ord` (deny-by-default) catches it; `cargo check` is silent.
- `assert_eq!(a, a)`, the test that passes whatever the code does. `clippy::eq_op` flags identical operands.
- Quadratic loops and needless clones: "code that can be written to run faster" is `clippy::perf`'s job. Neither check nor the test suite notices until the profiler does, months later.

Even rustc admits its own gap: per the [cargo book](https://doc.rust-lang.org/stable/cargo/commands/cargo-check.html), some diagnostics are only emitted during code generation, which `cargo check` skips. That is why the CI gate is `cargo clippy ... -D warnings` (`.github/workflows/ci.yml`), and why this repo's own tests carry deliberate `#[allow(clippy::field_reassign_with_default)]` suppressions in `src/tests/onboarding_*_test.rs`: the lint fires here, and each allow is a conscious call, not an oversight.

A codebase built with `cargo check` alone can report *thousands* of clippy findings on its first run, so the time to start is the first commit. If you're inheriting a backlog, `cargo clippy --fix` mechanically applies the machine-applicable suggestions, so the wall is smaller than it looks.

### Running the App While You Iterate

**Use `cargo run`. Do NOT `cargo build --release` for normal development.**

```bash
cargo run --all-features                 # default debug build, fastest iteration
cargo run --all-features -- /sessions    # pass CLI flags after `--`
cargo run --all-features -- -p hermes    # named profile
```

`cargo run` compiles a debug build and launches the TUI directly. Debug builds compile ~5x faster than release, and the extra instrumentation (full backtraces, debug assertions) makes any regression you introduce surface immediately. The debug binary is what you want when chasing a bug — release optimizations strip frames that make panics impossible to read.

**Brain files just work.** `cargo run` resolves `~/.opencrabs/` (or `~/.opencrabs/profiles/<name>/` with `-p`) the same way the installed binary does. Your existing config, brain files, sessions, and memory are all picked up — no copy step, no env var override needed.

#### `target/` Grows Fast on the Debug Loop

The `cargo run` loop is cheap in time and expensive in disk. Nothing in the toolchain prunes stale artifacts, so `target/` accumulates across every build, every feature combination, and every dependency bump. A maintainer checkout measured **33 GB** in September 2026: 29 GB in `target/debug`, 4.3 GB from a separate CI profile.

`--all-features` is the multiplier. The local STT/TTS, `pdfium`, and browser dependency trees are large, and building them debug-mode with full debug info is what fills the volume.

Keep it under control:

```bash
du -sh target                  # check before it surprises you
cargo clean                    # nuke the whole tree (next build is a full rebuild)
cargo clean -p opencrabs       # drop only this crate's artifacts, keep deps compiled
```

`cargo clean -p opencrabs` is the one you usually want: it keeps the dependency graph compiled, so the next `cargo run` recompiles our crate only and takes seconds instead of minutes.

If your home volume is small, move the tree somewhere bigger instead of cleaning it constantly:

```bash
export CARGO_TARGET_DIR=/path/to/big/volume/opencrabs-target
```

Put that in your shell profile and every `cargo run` / `cargo test` in the repo writes there instead. Release builds deserve the same treatment: run `cargo clean` before `cargo build --release` so a stale debug tree is not sitting next to the release artifacts you are about to produce.

#### When You Actually Want a Release Build

Reach for `cargo build --release` only when:

- You want to **test the optimized binary** (perf-sensitive feature work, startup time benchmarks, etc.).
- You want to **replace your installed binary with the latest from `main`** so the long-running daemon / channels keep running the new code. The flow:
  ```bash
  git pull origin main
  cargo build --release --all-features
  # macOS / Linux: replace ~/.cargo/bin/opencrabs (or wherever your install lives)
  cp target/release/opencrabs ~/.cargo/bin/opencrabs
  ```
  The release binary reads the same `~/.opencrabs/` as `cargo run`, so your brain files, sessions, and config carry over seamlessly.

For everything else — bug hunting, feature work, test-driven development — stay on `cargo run`. Release builds are a deployment step, not a dev loop.

### Where Tests Live

**Every test goes under `src/tests/` as a dedicated `*_test.rs` file**, registered in `src/tests/mod.rs`. No inline `#[cfg(test)] mod tests { ... }` blocks at the bottom of source files — they are explicitly forbidden by project policy because they hide behind the source file in IDE outlines and grow unbounded.

To add a new test file:

```bash
# 1. Create the test file
$EDITOR src/tests/my_feature_test.rs

# 2. Register it in src/tests/mod.rs (alphabetical-ish neighbourhood)
echo "pub mod my_feature_test;" >> src/tests/mod.rs   # or insert manually

# 3. If the test needs internal helpers from the module under test,
#    bump those helpers from `fn` / `pub(super)` to `pub(crate)` so the
#    test file can reach them without weakening the public API.

# 4. Verify
cargo test --all-features my_feature_test
```

If you find an existing inline `#[cfg(test)] mod tests` while working on a file, move it into `src/tests/` as part of your change. Leaving the violation in place will fail review.

### Tests Never Touch the Live Config or Keys

**No test may write `~/.opencrabs/config.toml`, `keys.toml`, or anything else in the live default home.** Any test that reaches a config or keys writer (the onboarding wizard's save, `Config::write_key`, `Config::write_array`, `save_keys`, a migration) must run under a home override so the write lands in a throwaway directory:

```rust
let dir = tempfile::tempdir().unwrap();
let home = dir.path().join(".opencrabs");
std::fs::create_dir_all(&home).unwrap();
crate::config::profile::with_home_override(home, || {
    // drive the wizard / call the writer here
});
```

A test binary logs nowhere, so a leaking test silently rewrites the developer's real settings with fixture defaults. Two wizard tests did exactly that and disabled voice on every `cargo test` run for an evening before anyone found the writer (#1399). `atomic_write` now refuses a path directly in the live default home under `cfg(test)`; if your test hits that refusal, the test is wrong, not the guard. Never disable or work around it.

### Commit Discipline — Atomic Commits

**Always do atomic commits:** repository-wide atomic commits (grouping all files changed for a single logical change) combined with short-lived feature branches or stacked pull requests.

Repository-wide means the unit is the logical change, not the file. Every file a change touches lands in the same commit, so the tree builds and the tests pass at every commit and a revert or bisect can land on a single sha. Three unrelated edits in one file are three commits; one change spread across ten files is one commit. Each commit branches off `main` on a short-lived branch and lands as its own PR, or as one PR in a stack when later commits depend on earlier ones.

- **Don't bundle** `cargo fmt` drift with feature work. Run fmt in its own commit (`chore: cargo fmt`).
- **Don't bundle** rename / move / restructure with logic changes. The reviewer cannot tell what's mechanical and what's behavioural.
- **Split test additions from production fixes only if the test would compile against the un-fixed code.** Otherwise commit them together so the test demonstrates the fix.
- **Commit message body explains the WHY**, not the diff. The diff already shows what changed; the message should answer "why was that wrong?" and "what would break if we reverted this?".
- **Never add `[skip ci]` to a commit.** The pipeline is fast and cheap to run; a skipped commit is an unverified commit, and on a release commit `[skip ci]` also skips the release workflow.
- **Never add `Co-Authored-By` lines** to commit messages. Project policy.

### Project Structure

```
src/
├── main.rs              # Binary entry point
├── lib.rs               # Crate root
├── app/                 # Application lifecycle / startup
├── brain/               # AI agent core
│   ├── agent/           # Agent orchestration, tool loop, context management
│   ├── provider/        # LLM providers (Anthropic, OpenAI-compatible, Copilot, custom)
│   ├── tools/           # Built-in tools (bash, edit, browser, memory_search, etc.)
│   ├── goal/            # Autonomous goal-completion loop
│   └── mission_control/ # Data services behind the Mission Control TUI panels
├── channels/            # Messaging + voice (Telegram, WhatsApp, Discord, Slack, Trello, voice)
├── a2a/                 # Agent-to-Agent protocol (agent card, JSON-RPC task API, HTTP gateway)
├── cli/                 # Command-line interface (Clap)
├── config/              # Configuration (config.toml + keys.toml)
├── cron/                # Cron scheduler — polls cron_jobs and runs due jobs
├── db/                  # SQLite database layer (SQLx)
├── error/               # Error types (OpenCrabsError, ErrorCode)
├── eval/                # Offline evaluation harness (context/memory quality; feature-gated)
├── logging/             # Conditional logging system
├── memory/              # Long-term memory (FTS5 + vector search via qmd)
├── migrations/          # SQLite migrations
├── rtk/                 # Rust Token Killer — compresses bash output to save context tokens
├── services/            # Business logic (Session, Message, File, Plan)
├── tui/                 # Terminal UI (ratatui)
├── usage/               # Usage analytics dashboard
├── utils/               # Shared utilities
├── tests/               # All tests, one *_test.rs per module (see TESTING.md)
├── benches/             # Criterion benchmarks
├── assets/              # Icons, screenshots, visual assets
├── scripts/             # Build and setup scripts
├── docker/              # Dockerfile + compose.yml
├── evals/               # Eval datasets / fixtures
└── docs/                # Documentation templates + reference/ architecture docs
```

## Coding Standards

### Rust Conventions

- **Files**: `snake_case.rs` — never PascalCase, never camelCase
- **Structs/Enums**: `PascalCase`
- **Functions/Variables**: `snake_case`
- **Constants**: `SCREAMING_SNAKE_CASE`
- **Error handling**: `anyhow::Result` for application errors, `thiserror` for typed errors
- **Async**: `tokio` runtime — never block in async functions
- **`mod.rs` is for module declarations ONLY — functions NEVER live in `mod.rs`. Ever.** A `mod.rs` file may contain exactly: the module doc comment, `mod`/`pub mod`/`pub(crate) mod` declarations, and `pub(crate) use` re-exports of the submodule surface. Zero `fn` definitions. When a function grows in `mod.rs`, that is the signal it belongs in a named submodule (`detect.rs`, `gate.rs`, whatever the cohesion says) — create the file, move the fn, re-export it from `mod.rs` so call sites don't change. Reference implementation: `src/channels/telegram/rich/mod.rs`. If you find functions in a `mod.rs` while working on it, moving them out is part of your change — leaving the violation in place will fail review.

### What We Value

- **Working code** over clever code
- **Minimal diffs** — change only what's needed
- **Tests that prove the fix** — not tests for the sake of coverage
- **Comments that explain why**, not what

### Commit Messages

Use [Conventional Commits](https://www.conventionalcommits.org/), the same shape as issue titles (see **Issue Titles** above):

```
feat: add voice message support for Discord channel
fix: prevent duplicate message rendering for CLI providers
refactor: simplify tool loop iteration tracking
```

## What Gets Your PR Closed

To be transparent, here's what will get your PR closed immediately:

- **Stub/placeholder code** — Empty implementations, `todo!()`, functions that return hardcoded empty values
- **No linked issue** — Feature PRs without an approved issue
- **Fails CI** — If `cargo fmt --check`, `cargo clippy`, or `cargo test` fail
- **Unrelated changes** — Reformatting files you didn't modify, drive-by "improvements"
- **No tests** — Bug fixes without a regression test, features without any tests
- **Tests that write the live config** — A test that saves `config.toml` or `keys.toml` outside a home override
- **AI-generated spam** — PRs that look like they were generated by an LLM with no understanding of the codebase

## Don't Know How to Code?

That's completely fine. You can still contribute meaningfully:

- **Report bugs** with detailed reproduction steps
- **Request features** with clear descriptions of the problem you're trying to solve
- **Improve documentation** — fix typos, clarify confusing sections, add examples
- **Test pre-release builds** and report issues
- **Answer questions** in GitHub Discussions

A well-written bug report or feature request is worth more than a stub PR. Seriously.

## License

By contributing to OpenCrabs, you agree that your contributions will be licensed under the MIT License. See [LICENSE.md](LICENSE.md) for details.
