#!/usr/bin/env bash
set -euo pipefail

revision_range="${1:?usage: check-commit-attribution.sh <revision-range>}"
identity_pattern='(^|[^[:alnum:]_])(bot|ai|agent|assistant|copilot|claude|chatgpt|openai|codex|cursor|gemini|devin|swe-agent)([^[:alnum:]_]|$)|\[bot\]|noreply\+[^[:space:]]*\[bot\]'
failed=0

while IFS= read -r commit; do
  [[ -n "$commit" ]] || continue
  message="$(git show -s --format=%B "$commit")"
  if printf '%s\n' "$message" | grep -Eiq "^[[:space:]]*Co-authored-by:[[:space:]].*${identity_pattern}"; then
    echo "Forbidden automated co-author trailer in commit $commit" >&2
    git show -s --format='  %s%n  %b' "$commit" >&2
    failed=1
  fi
done < <(git rev-list "$revision_range")

if [[ "$failed" -ne 0 ]]; then
  echo "Commit attribution policy failed. Remove AI/bot co-author trailers." >&2
  exit 1
fi

echo "Commit attribution policy passed for range: $revision_range"
