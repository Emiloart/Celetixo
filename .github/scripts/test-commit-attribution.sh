#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
project_root="$(cd -- "$script_dir/../.." && pwd)"
checker="$script_dir/check-commit-attribution.sh"
hook="$project_root/.githooks/commit-msg"
fixture_root="$(mktemp -d "${TMPDIR:-/tmp}/celetixo-attribution.XXXXXXXX")"
trap 'rm -rf -- "$fixture_root"' EXIT

# These settings apply only to this test process and its disposable repos.
# Inherited worktree/index overrides must never redirect a fixture commit
# into the contributor's checkout.
unset GIT_DIR GIT_WORK_TREE GIT_COMMON_DIR GIT_INDEX_FILE GIT_OBJECT_DIRECTORY
unset GIT_ALTERNATE_OBJECT_DIRECTORIES GIT_CONFIG GIT_CONFIG_PARAMETERS
unset GIT_CONFIG_COUNT GIT_NAMESPACE GIT_SHALLOW_FILE GIT_REPLACE_REF_BASE
unset GIT_TEMPLATE_DIR GIT_DEFAULT_HASH
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_SYSTEM=/dev/null GIT_CONFIG_GLOBAL=/dev/null
export GIT_AUTHOR_NAME='Alice Example' GIT_AUTHOR_EMAIL='alice@example.test'
export GIT_COMMITTER_NAME='Human Maintainer' GIT_COMMITTER_EMAIL='maintainer@example.test'
export GIT_AUTHOR_DATE='2000-01-01T00:00:00Z' GIT_COMMITTER_DATE='2000-01-01T00:00:00Z'
export GIT_TERMINAL_PROMPT=0

repo="$fixture_root/repo"
empty_hooks="$fixture_root/no-hooks"
active_hooks="$fixture_root/active-hooks"
mkdir -p "$empty_hooks" "$active_hooks" "$fixture_root/templates"
git init --quiet --template="$fixture_root/templates" "$repo"
git -C "$repo" config user.name 'Alice Example'
git -C "$repo" config user.email 'alice@example.test'
git -C "$repo" config commit.gpgSign false
cp "$hook" "$active_hooks/commit-msg"
chmod +x "$active_hooks/commit-msg"
log="$fixture_root/result.log"
passed=0

fail() {
  echo "FAIL: $*" >&2
  cat "$log" >&2
  exit 1
}

expect_pass() {
  local label="$1"
  shift
  if ! "$@" >"$log" 2>&1; then
    fail "$label"
  fi
  passed=$((passed + 1))
  echo "PASS: $label"
}

expect_rejection() {
  local label="$1" diagnostic="$2"
  shift 2
  if "$@" >"$log" 2>&1; then
    fail "$label unexpectedly succeeded"
  fi
  if ! grep -Fq -- "$diagnostic" "$log"; then
    fail "$label did not report the expected reason: $diagnostic"
  fi
  passed=$((passed + 1))
  echo "PASS: $label"
}

fixture_commit() {
  git -C "$repo" -c core.hooksPath="$empty_hooks" commit --quiet --allow-empty "$@"
  latest="$(git -C "$repo" rev-parse HEAD)"
}

check_latest() {
  (cd -- "$repo" && bash "$checker" "$latest^!")
}

check_range() {
  (cd -- "$repo" && bash "$checker" "$1")
}

hook_commit() {
  git -C "$repo" -c core.hooksPath="$active_hooks" commit --quiet --allow-empty "$@"
}

expect_hook_rejection() {
  local label="$1" diagnostic="$2" before after
  shift 2
  before="$(git -C "$repo" rev-parse HEAD)"
  expect_rejection "$label" "$diagnostic" hook_commit "$@"
  after="$(git -C "$repo" rev-parse HEAD)"
  [[ "$before" == "$after" ]] || fail "$label advanced HEAD after rejection"
}

fixture_commit -m 'Human-authored change' -m 'Co-authored-by: Bob Human <bob@example.test>'
expect_pass 'CI permits human author, committer, and co-author' check_latest

fixture_commit -m 'Document AI and [bot] integration' -m 'Compare Codex, OpenAI, Claude and other assistants in engineering records.'
expect_pass 'CI permits ordinary AI and [bot] body mentions' check_latest

GIT_AUTHOR_NAME='Aileen Agenton' GIT_AUTHOR_EMAIL='robotics@example.test' fixture_commit -m 'Partial words are human identity tokens'
expect_pass 'CI does not match automation substrings inside human words' check_latest

GIT_AUTHOR_NAME='Codex' fixture_commit -m 'Synthetic invalid author fixture'
expect_rejection 'CI rejects automated author name' 'Forbidden automated author identity' check_latest

GIT_AUTHOR_EMAIL='codex@openai.com' fixture_commit -m 'Synthetic invalid author address fixture'
expect_rejection 'CI rejects automated author address' 'Forbidden automated author identity' check_latest

GIT_COMMITTER_NAME='Build [bot]' fixture_commit -m 'Synthetic invalid committer fixture'
expect_rejection 'CI rejects automated committer name' 'Forbidden automated committer identity' check_latest

GIT_COMMITTER_EMAIL='assistant@example.test' fixture_commit -m 'Synthetic invalid committer address fixture'
expect_rejection 'CI rejects automated committer address' 'Forbidden automated committer identity' check_latest

fixture_commit -m 'Synthetic invalid co-author fixture' -m 'Co-authored-by: Codex <codex@openai.com>'
expect_rejection 'CI rejects automated co-author' 'Forbidden automated co-author trailer' check_latest

fixture_commit -m 'Synthetic compact attribution fixture' -m 'cO-AuThOrEd-By:chatgpt <chatgpt@example.test>'
expect_rejection 'CI rejects compact case-insensitive attribution' 'Forbidden automated co-author trailer' check_latest

expect_rejection 'CI fails an invalid revision range' 'could not resolve revision range' check_range 'missing-attribution-fixture..HEAD'
expect_rejection 'CI rejects option injection as a range' 'expected a revision range, not an option' check_range '--all'
expect_pass 'CI accepts an empty valid revision range' check_range 'HEAD..HEAD'

printf '%s\n' 'Alice Example <alice@example.test> Codex <codex@example.test>' >"$repo/.mailmap"
git -C "$repo" config log.mailmap true
GIT_AUTHOR_NAME='Codex' GIT_AUTHOR_EMAIL='codex@example.test' fixture_commit -m 'Synthetic mailmap author fixture'
expect_rejection 'CI checks raw author identity despite mailmap' 'Forbidden automated author identity' check_latest
GIT_COMMITTER_NAME='Codex' GIT_COMMITTER_EMAIL='codex@example.test' fixture_commit -m 'Synthetic mailmap committer fixture'
expect_rejection 'CI checks raw committer identity despite mailmap' 'Forbidden automated committer identity' check_latest

# The forbidden attribution occurs before a large, valid trailer block.
# printf | grep -q under pipefail can misclassify this as success when the
# producer receives SIGPIPE after grep finds the first attribution.
large_message="$fixture_root/large-message.txt"
{
  printf '%s\n\n' 'Synthetic large attribution fixture'
  printf '%s\n' 'Co-authored-by: Codex <codex@openai.com>'
  for ((i = 0; i < 12000; i++)); do
    printf '%s\n' 'Reviewed-by: Human Reviewer <reviewer@example.test>'
  done
} >"$large_message"
fixture_commit -F "$large_message"
expect_rejection 'CI rejects automated co-author in a large message' 'Forbidden automated co-author trailer' check_latest

expect_pass 'actual hook permits human attribution and AI body discussion' hook_commit -m 'Document AI and [bot] integrations' -m 'Codex assistance is recorded in engineering evidence.' -m 'Co-authored-by: Bob Human <bob@example.test>'
expect_hook_rejection 'actual hook rejects --author override' 'must not be Git authors' --author='Codex <codex@openai.com>' -m 'Synthetic override fixture'
GIT_AUTHOR_EMAIL='codex@openai.com' expect_hook_rejection 'actual hook rejects author environment address' 'must not be Git authors' -m 'Synthetic author environment fixture'
GIT_COMMITTER_NAME='Assistant' expect_hook_rejection 'actual hook rejects committer environment' 'must not be Git committers' -m 'Synthetic committer environment fixture'
expect_hook_rejection 'actual hook rejects automated co-author' 'must not be Git co-authors' -m 'Synthetic co-author fixture' -m 'Co-authored-by: Codex <codex@openai.com>'
expect_hook_rejection 'actual hook rejects compact automated co-author' 'must not be Git co-authors' -m 'Synthetic compact attribution fixture' -m 'Co-authored-by:Codex <codex@example.test>'
expect_hook_rejection 'actual hook rejects a large automated co-author message' 'must not be Git co-authors' -F "$large_message"
expect_pass 'actual hook permits a human --author override' hook_commit --author='Bob Human <bob@example.test>' -m 'Human override fixture'
[[ "$(git -C "$repo" show -s --format='%an <%ae>' HEAD)" == 'Bob Human <bob@example.test>' ]] || fail 'human author override was not preserved'
expect_pass 'CI accepts actual-hook human override commit' check_range 'HEAD^!'
expect_rejection 'hook fails when its message file is missing' 'commit message file not found' bash "$hook" "$fixture_root/missing-message.txt"

echo "Commit attribution regression fixtures passed: $passed checks."
