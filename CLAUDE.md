# CLAUDE.md

Torch (working name): a native, open-source IDE for AI-augmented coding.
`docs/VISION.md` is the source of truth for goals, interaction model, pillars, AI rules and
architecture. Design decisions are in `docs/decisions/`.

## Layout

- `crates/torch-core`: all domain logic (decision log, rules, criticality, `ChangeStore`).
- `crates/torch-cli`: `torch` binary and MCP server; thin layer over core.
- `crates/torch-app`: GPUI desktop app; thin layer over core.

## Commands

```sh
cargo build                                  # build everything
cargo test --workspace                       # run tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all                              # format (CI runs --check)
cargo run -p torch-app                       # open the app
cargo run -p torch-cli -- <args>             # run the CLI (binary name: torch)
```

Linux needs GPUI's system libraries (see `.github/workflows/ci.yml` for the apt list).

## Working rules

- Show a plan and wait for the user's OK before writing code for each phase.
- Small, reviewable commits with clear messages. Never push, merge or rebase.
- Don't read or write outside this repo.
- Prefer well-known crates. Ask before adding a dependency.
- Write tests for core logic as you go. Run `cargo clippy` and `cargo fmt` before each commit.
- Record design decisions as short ADRs in `docs/decisions/` (context, decision,
  alternatives rejected).
- The user is a senior .NET developer newer to Rust: explain non-obvious Rust choices
  (ownership, lifetimes, async) briefly in commit messages or ADRs, not long chat replies.

## Technical constraints

- Everything is Rust. UI is GPUI only; never copy code from the Zed editor (GPL-3.0).
- Snappy is non-negotiable: no blocking work on the UI thread, heavy work in background
  tasks. Anything over 16 ms on the UI thread is a bug.
- Language intelligence comes from LSP and tree-sitter. No custom parsers.
- All logic lives in `torch-core`. All git access goes through the `ChangeStore` trait.
- GPUI is pinned to a Zed git rev (ADR 0002). Bump rev and `rust-toolchain.toml` together.
