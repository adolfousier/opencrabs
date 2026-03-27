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

# Build with all features
cargo clippy --all-features

# Run the exact CI checks (you MUST pass all three before submitting a PR)
cargo fmt --all -- --check
cargo clippy --lib --bins --tests --all-features -- -D warnings
cargo test --all-features --verbose
```

**All three commands must pass.** PRs that fail CI will not be reviewed.

### Project Structure

```
src/
├── brain/           # AI agent core
│   ├── agent/       # Agent orchestration, tool loop, context management
│   ├── provider/    # LLM provider implementations (Anthropic, OpenAI, etc.)
│   └── tools/       # Built-in tools (bash, edit, browser, etc.)
├── channels/        # Communication channels (Telegram, Discord, Slack, WhatsApp)
├── config/          # Configuration management
├── database/        # SQLite database layer
├── memory/          # Long-term memory (FTS5 + vector search)
├── tui/             # Terminal UI (ratatui)
└── utils/           # Shared utilities
```

## Coding Standards

### Rust Conventions

- **Files**: `snake_case.rs` — never PascalCase, never camelCase
- **Structs/Enums**: `PascalCase`
- **Functions/Variables**: `snake_case`
- **Constants**: `SCREAMING_SNAKE_CASE`
- **Error handling**: `anyhow::Result` for application errors, `thiserror` for typed errors
- **Async**: `tokio` runtime — never block in async functions

### What We Value

- **Working code** over clever code
- **Minimal diffs** — change only what's needed
- **Tests that prove the fix** — not tests for the sake of coverage
- **Comments that explain why**, not what

### Commit Messages

Use [Conventional Commits](https://www.conventionalcommits.org/):

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

By contributing to OpenCrabs, you agree that your contributions will be licensed under the MIT License. See [LICENSE](LICENSE) for details.
