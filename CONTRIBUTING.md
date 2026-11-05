# Contributing to Celetixo

## Commit attribution

Git author, committer, and co-author identities must represent human contributors.

- Use the human contributor's established Git identity for authorship and committer metadata. Do not add AI tools, coding agents, automated assistants, or bots as Git authors, committers, or co-authors.
- Do not add `Co-authored-by` trailers that identify an AI system, agent, bot, or automated service.
- Human co-authors may be credited with their consent and their actual Git identity.
- AI-assisted implementation does not require a Git co-author trailer.
- This policy concerns Git attribution metadata. It does not prohibit accurate disclosure of AI assistance in engineering records where appropriate.

### Install local enforcement

After cloning the repository, run this once from the repository root using Git Bash:

```sh
bash scripts/install-git-hooks.sh
```

The installer marks `.githooks/commit-msg` executable and configures this checkout to use `.githooks`. The hook checks the actual author and committer identities prepared by Git, including an author supplied through `git commit --author`, and rejects recognized automated identities. It also checks lines declaring `Co-authored-by` attribution. Ordinary mentions of AI tools in the subject or body are allowed. The hook does not rewrite commit messages or silently remove attribution.

Git hooks are local safeguards and can be bypassed by changing local Git configuration or explicitly disabling hooks. The CI workflow independently checks raw author and committer metadata and co-author attribution on pushes and pull requests; mailmap aliases do not hide the original identities. Branch protection must require the attribution check for the policy to be mandatory before merging.

Recognition uses a maintained set of automation names and word boundaries. It cannot authenticate a human identity or establish a co-author's consent, and a matching human name may require maintainer review. Contributors remain responsible for accurate attribution.

Run the enforcement regression fixtures with:

```sh
bash .github/scripts/test-commit-attribution.sh
```

The fixtures create and remove isolated temporary Git repositories, including deliberately invalid commits. They do not create commits in this checkout or modify global Git configuration.
