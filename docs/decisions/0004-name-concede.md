# 0004. Name the product Concede

- Status: accepted
- Date: 2026-11-24

## Context

The project started under the working name "Torch". VISION.md listed the final name as an
open question. The repository directory is already called `concede`.

## Decision

The product is called **Concede**. Everything follows that name:

- Crates: `concede-core`, `concede-cli`, `concede-app` (Rust paths `concede_core`, ...).
- CLI binary: `concede` (e.g. `concede why <file>`, `concede inbox`).
- Per-repo data directory: `.concede/` (e.g. `.concede/decisions/`, `.concede/rules`).

The rename was done before any user data or release existed, so existing docs, including
earlier ADRs and the kickoff prompt, were updated in place, not superseded. The only
remaining "torch" is the unrelated earlier project `foa-torch`.

## Alternatives rejected

- **Keep "Torch".** It was only a working name, and the owner chose Concede.
- **Rename later.** Renaming gets more expensive once `.concede/` data exists in user
  repos and the CLI name is in hook configs and agent prompts.
