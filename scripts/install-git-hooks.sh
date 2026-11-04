#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel 2>/dev/null)" || {
  echo "Run this script from inside a Celetixo Git checkout." >&2
  exit 1
}

hook_dir="$repo_root/.githooks"
hook_file="$hook_dir/commit-msg"

if [[ ! -f "$hook_file" ]]; then
  echo "Required hook not found: $hook_file" >&2
  exit 1
fi

chmod +x "$hook_file"
git -C "$repo_root" config --local core.hooksPath .githooks

echo "Celetixo commit-msg hook installed for this checkout."
echo "Configured core.hooksPath: $(git -C "$repo_root" config --local --get core.hooksPath)"
