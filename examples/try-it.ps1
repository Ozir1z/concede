#!/usr/bin/env pwsh
#
# Set up a scratch repo to try Concede with Claude Code end to end (Windows twin
# of try-it.sh). Copies the C# sample into scratch/<name>, makes it a git repo,
# wires in the Claude Code hook + MCP config, builds the `concede` binary, and
# prints the exact commands to run. It does not start Claude Code itself.
#
# Usage: examples/try-it.ps1 [name]   (default name: sample)

$ErrorActionPreference = 'Stop'

$name = if ($args.Count -ge 1) { $args[0] } else { 'sample' }
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$dest = Join-Path $repoRoot "scratch/$name"

Write-Host '==> Building the concede binary'
cargo build -p concede-cli --quiet --manifest-path (Join-Path $repoRoot 'Cargo.toml')
$binDir = Join-Path $repoRoot 'target/debug'

if (Test-Path $dest) {
    Write-Error "error: $dest already exists; remove it or pick another name"
}

Write-Host "==> Creating scratch repo at scratch/$name"
New-Item -ItemType Directory -Path $dest | Out-Null
Copy-Item -Recurse -Force (Join-Path $repoRoot 'examples/sample-csharp/*') $dest

# Wire in the Claude Code hook + MCP config (dotfiles included).
Copy-Item -Recurse -Force (Join-Path $repoRoot 'examples/claude-code/*') $dest
Copy-Item -Recurse -Force (Join-Path $repoRoot 'examples/claude-code/.claude') $dest
Copy-Item -Force (Join-Path $repoRoot 'examples/claude-code/.mcp.json') $dest

Write-Host '==> Initialising git (captures diff against HEAD, so we need a commit)'
git -C $dest init --quiet
git -C $dest add -A
git -C $dest -c user.name='Concede Demo' -c user.email='demo@example.com' commit --quiet -m 'Import C# sample'

@"

Scratch repo ready: scratch/$name

Run these from there, with the built concede on PATH:

  cd "$dest"
  `$env:PATH = "$binDir" + [IO.Path]::PathSeparator + `$env:PATH

  # 1. Make a change in a Claude Code session. Haiku is fine. In a fresh,
  #    untrusted folder Claude Code ignores the project's allow rules, so pass
  #    the tools explicitly (the capture hook runs regardless):
  claude -p "Add an order cancellation feature: a Cancel() method on Order and Cancel(order) on OrderService, with tests. Record your reasoning with record_decision." ``
    --allowedTools mcp__concede__record_decision mcp__concede__why

  # 2. See what hasn't been reviewed yet:
  concede inbox

  # 3. See which decisions touched a file, with reasoning:
  concede why src/Order.cs
"@ | Write-Host
