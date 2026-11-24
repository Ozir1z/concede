# Concede

A free, open-source, native IDE for AI-augmented coding.

AI writes most code now, but developers lose agency: agents produce more than anyone wants
to review, and the reasoning behind each change disappears when the session ends. Concede
puts the human back in the driver's seat as navigator, reviewer and decision maker.

- **Visual first.** Code Map, story inbox and decision timelines; one key zooms into code.
- **Decision log.** Every AI change records *why*, in the repo, queryable per file and line.
- **Change Stories.** AI output grouped by intent and ordered by dependency, reviewed per group.
- **AI Rules.** Per-action, per-zone authority for each model; critical changes need sign-off.
- **Fast.** Rust and GPUI, GPU-rendered. Keyboard-first, with optional vim bindings.
- **Open.** Local-first, bring your own models, built on git, LSP and tree-sitter.

Vision: [docs/VISION.md](docs/VISION.md). Design decisions: [docs/decisions/](docs/decisions/).

Status: the decision log works from the CLI and Claude Code ([setup](docs/capture.md)); the app is a
scaffold. Build with `cargo build`; run the app with `cargo run -p concede-app`.

Licensed under [Apache-2.0](LICENSE).
