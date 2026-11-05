#!/usr/bin/env bash
set -euo pipefail

revision_range="${1:?usage: check-commit-attribution.sh <revision-range>}"
identity_pattern='(^|[^[:alnum:]_])(bot|ai|agent|assistant|copilot|claude|chatgpt|openai|codex|cursor|gemini|devin|swe-agent)([^[:alnum:]_]|$)|\[bot\]|noreply\+[^[:space:]]*\[bot\]'
failed=0

# Do not let an option masquerade as a range. Capture rev-list's status
# explicitly: process substitution would hide an invalid revision's failure.
if [[ "$revision_range" == -* ]]; then
  echo "Commit attribution policy failed: expected a revision range, not an option." >&2
  exit 1
fi
if ! commits="$(git rev-list "$revision_range" --)"; then
  echo "Commit attribution policy failed: could not resolve revision range." >&2
  exit 1
fi

while IFS= read -r commit; do
  [[ -n "$commit" ]] || continue
  # Lowercase placeholders inspect raw identities, never mailmap aliases.
  author="$(git show -s --format='%an <%ae>' "$commit" --)"
  committer="$(git show -s --format='%cn <%ce>' "$commit" --)"
  for role in author committer; do
    if grep -Eiq "$identity_pattern" <<<"${!role}"; then
      echo "Forbidden automated ${role} identity in commit $commit: ${!role}" >&2
      failed=1
    fi
  done
  message="$(git show -s --format=%B "$commit" --)"
  if grep -Eiq "^[[:space:]]*Co-authored-by:[[:space:]]*.*(${identity_pattern})" <<<"$message"; then
    echo "Forbidden automated co-author trailer in commit $commit" >&2
    failed=1
  fi
done <<<"$commits"

if [[ "$failed" -ne 0 ]]; then
  echo "Commit attribution policy failed. Authors, committers, and co-authors must be human contributors." >&2
  exit 1
fi

echo "Commit attribution policy passed for range: $revision_range"
