#!/usr/bin/env bash
#
# Set up a scratch repo to try Concede with Claude Code end to end.
#
# It copies the C# sample into scratch/<name>, makes it a git repo, wires in the
# Claude Code hook + MCP config, builds the `concede` binary, and prints the exact
# commands to run. It does not start Claude Code itself.
#
# Usage: examples/try-it.sh [name]   (default name: sample)

set -euo pipefail

name="${1:-sample}"
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dest="$repo_root/scratch/$name"

echo "==> Building the concede binary"
cargo build -p concede-cli --quiet --manifest-path "$repo_root/Cargo.toml"
bin_dir="$repo_root/target/debug"

if [ -e "$dest" ]; then
  echo "error: $dest already exists; remove it or pick another name" >&2
  exit 1
fi

echo "==> Creating scratch repo at scratch/$name"
mkdir -p "$dest"
cp -R "$repo_root/examples/sample-csharp/." "$dest/"

# Wire in the Claude Code hook + MCP config (dotfiles included).
cp -R "$repo_root/examples/claude-code/." "$dest/"

echo "==> Initialising git (captures diff against HEAD, so we need a commit)"
git -C "$dest" init --quiet
git -C "$dest" add -A
git -C "$dest" -c user.name="Concede Demo" -c user.email="demo@example.com" \
  commit --quiet -m "Import C# sample"

cat <<EOF

Scratch repo ready: scratch/$name

Run these from there, with the built concede on PATH:

  cd "$dest"
  export PATH="$bin_dir:\$PATH"

  # 1. Make a change in a Claude Code session. Haiku is fine. In a fresh,
  #    untrusted folder Claude Code ignores the project's allow rules, so pass
  #    the tools explicitly (the capture hook runs regardless):
  claude -p "Add an order cancellation feature: a Cancel() method on Order and \
Cancel(order) on OrderService, with tests. Record your reasoning with record_decision." \
    --allowedTools mcp__concede__record_decision mcp__concede__why

  # 2. See what hasn't been reviewed yet:
  concede inbox

  # 3. See which decisions touched a file, with reasoning:
  concede why src/Order.cs
EOF
