# 0002. Depend on GPUI via a pinned git revision of the Zed repo

- Status: accepted
- Date: 2026-11-24

## Context

`torch-app` is built on GPUI, Zed's GPU-accelerated UI framework (Apache-2.0). As of
2026-11-24:

- The official `gpui` crate on crates.io (owned by zed-industries) is at 0.2.2, published
  2025-10-22. Zed's `main` still declares 0.2.2 but has a year of changes on top, so the
  crates.io release is badly out of date.
- Since then, windowing/text backends live in a separate `gpui_platform` crate that is
  needed to start an application (`gpui_platform::application()`).
- Several third-party republications exist: `gpui-ce` (community fork), `gpui-pre`
  (snapshots published by the gpui-component maintainer), `gpui-fast` (Longbridge fork with
  rendering changes).

GPUI is pre-1.0 and breaks often. Its README says to use the latest stable Rust.

## Decision

Depend on `gpui` and `gpui_platform` as git dependencies on
`https://github.com/zed-industries/zed`, pinned with `rev = "<sha>"` in
`[workspace.dependencies]`. Pin `rust-toolchain.toml` to the toolchain Zed uses at that
commit. Upgrades are deliberate: bump the rev and toolchain together in one commit and fix
breakage there.

Initial pin: `c016f436e60bfa5105de53837935e9de94eb0867` (Zed main, 2026-11-24), Rust 1.100.0.

## Alternatives rejected

- **crates.io `gpui` 0.2.2.** Official, but a year behind and missing `gpui_platform`.
- **Git dependency tracking a branch.** Not reproducible; every `cargo update` could break us.
- **Community forks (`gpui-ce`, `gpui-pre`, `gpui-fast`).** Add a third-party maintainer
  between us and our most critical dependency.
- **Vendoring GPUI.** Large and costly to maintain; nothing to gain while upstream is healthy.

## Consequences

- First build clones the full Zed repo and compiles a lot of code; CI relies on caching.
- `Cargo.lock` is committed so the transitive graph is reproducible.
- Only GPUI crates are used. No Zed editor code (GPL-3.0) is copied.

## License audit (at the initial pin)

Cargo clones the whole Zed repo but compiles only GPUI and the crates it needs. All 20
Zed-repo crates in our graph are Apache-2.0: `gpui`, `gpui_platform`, `gpui_{apple,ios,
linux,macos,web,wgpu,windows}`, `gpui_macros`, `gpui_shared_string`, `gpui_util`,
`collections`, `refineable`, `derive_refineable`, `http_client`, `scheduler`, `sum_tree`,
`ztracing`, `ztracing_macro`. Across all third-party crates there is no GPL-only license;
the few copyleft-capable ones are dual-licensed with a permissive option, or MPL-2.0
(file-level, compatible with Apache-2.0 distribution). Re-check on every rev bump:

```sh
cargo metadata --format-version 1 | jq -r '.packages[] | select(.source // "" | test("zed-industries/zed")) | "\(.name) \(.license)"'
```
