# Continue Concede: Phase 1, step 6

Paste everything below the line into Claude Code at the root of the `concede` repository.

---

We're building **Concede**, a free, open-source, native IDE for AI-augmented coding, in Rust
with GPUI. Read these first, in this order; they are the source of truth:

1. `CLAUDE.md`: commands, layout and working rules (follow them).
2. `docs/VISION.md`: goals, interaction model, pillars.
3. `docs/KICKOFF.md`: the phased roadmap (Phase 0, 1, 2, later).
4. `docs/decisions/`: ADRs 0001–0007. 0005–0007 define the decision log.
5. `docs/capture.md`: how Claude Code feeds the log (hook + MCP).

## Where we are

Phase 0 is done (workspace, GPUI window, CI green on Linux, macOS and Windows).

Phase 1 (decision log) steps 1–5 are done and pushed to `main`:

1. ADRs 0005–0007 accepted (schema/storage, ChangeStore via git CLI, capture via hooks + MCP).
2. `concede-core`: record types, validation, `Store` (`.concede/{decisions,events,captures}/<ULID>.json`,
   immutable, atomic writes), `Log` queries, JSON Schema in `docs/schema/`.
3. `ChangeStore` trait + `GitCli`, line-drift mapping (`linemap`), and `Concede` with
   `record_capture`, `record_decision` (claims captures), `review`, `link`, `why`.
4. `concede` CLI: `log`, `why <file>[:lines]`, `inbox`, `review`, `link`, `capture`.
5. `concede mcp`: MCP server (rmcp) with `record_decision` and `why`; example configs in
   `examples/claude-code/`; setup docs in `docs/capture.md`.

About 85 tests (`cargo test --workspace`), clippy and fmt clean.

## Next: step 6, the Phase 1 "done" check

Goal from the kickoff: *I can run a Claude Code session in a sample repo, then run
`concede why src/SomeFile.cs` and see which decisions touched it, with reasoning, and
`concede inbox` shows what I haven't reviewed.*

Plan (show me the plan and wait for my OK before writing code, as usual):

- Add a small C# sample under `examples/sample-csharp/` (a few files, e.g. an `Order` with
  `Submit`, an `OrderService`, a test file). It doesn't need to build in CI.
- Add a short script (`examples/try-it.sh`, plus a PowerShell twin if cheap) that copies the
  sample into `scratch/<name>`, runs `git init`, copies `examples/claude-code/.` into it, and
  prints the exact `claude` command to run there, followed by `concede inbox` and
  `concede why src/Order.cs`.
- Run it once yourself end to end with `claude -p` (Haiku is fine) and report the output.
- Then mark Phase 1 done in a short note, and propose the Phase 2 plan (visual mode shell:
  story inbox in `concede-app`, keyboard-first, vim mode, frame-time logging). The kickoff
  says to evaluate `gpui-component` at the start of Phase 2 (ADR 0003).

## Things we learned that aren't obvious from the code

- **Toolchain:** rustup installs to `~/.cargo/bin`. If `cargo` isn't found, prefix commands
  with `export PATH=$HOME/.cargo/bin:$PATH`. `rust-toolchain.toml` pins Rust 1.100.0 (matches
  the pinned Zed commit for GPUI). On Linux, GPUI needs system libraries (see the apt list in
  `.github/workflows/ci.yml`; on Arch/CachyOS the build just worked).
- **Feature unification:** GPUI turns on `serde_json/preserve_order` for the whole workspace,
  so anything that depends on JSON key order must sort explicitly (see `schema.rs`).
- **Windows:** `.gitattributes` forces LF; tests compare generated files ignoring CRLF.
  Don't canonicalise the git root (Windows gives `\\?\` paths git dislikes); canonicalise
  only when comparing paths.
- **Claude Code hook payloads** (checked against real runs, fixtures in
  `crates/concede-core/tests/fixtures/claude-code/`): `structuredPatch` includes context
  lines; `Write` creating a file sends an empty patch; there's `session_id` but no model.
- **Session matching:** Claude Code sets `CLAUDE_CODE_SESSION_ID` for MCP servers, and it
  equals the hook payload's `session_id` (verified), so claims are exact per session.
- **Permissions:** Claude Code ignores a project's permission `allow` rules until the folder
  is trusted. In `claude -p` in a fresh folder, pass
  `--allowedTools mcp__concede__record_decision mcp__concede__why` (hooks run regardless).
- **Testing with real agents:** use `/scratch/` (git-ignored). When piping JSON through zsh,
  use `printf '%s'`, not `echo` (zsh's echo expands `\n` and breaks the JSON).
- **GitHub CI logs** need a signed-in user; `gh` isn't installed on the original machine.
  The public API still shows job/step status:
  `https://api.github.com/repos/Ozir1z/concede/actions/runs?per_page=1`.

## Working rules (short version; full list in CLAUDE.md)

- Plan first, wait for my OK per phase/step. Small commits with clear messages, and Rust
  notes for me (I'm a senior .NET dev, newer to Rust) in commit messages or ADRs.
- Never push, merge or rebase unless I say so. Ask before adding a dependency.
- Record design decisions as ADRs. Run `cargo clippy` and `cargo fmt` before each commit.
