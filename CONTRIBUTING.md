# Contributing to Celetixo

## Commit attribution

Git commit authorship and co-authorship must represent human contributors.

- Do not add AI tools, coding agents, automated assistants, or bots as Git authors or co-authors.
- Do not add `Co-authored-by` trailers that identify an AI system, agent, bot, or automated service.
- Human co-authors may be credited with their consent and their actual Git identity.
- AI-assisted implementation does not require a Git co-author trailer.
- This policy concerns Git attribution metadata. It does not prohibit accurate disclosure of AI assistance in engineering records where appropriate.

### Install local enforcement

After cloning the repository, run this once from the repository root using Git Bash:

```sh
bash scripts/install-git-hooks.sh
```

The installer marks `.githooks/commit-msg` executable and configures this checkout to use `.githooks`. The hook rejects recognized automated identities in `Co-authored-by` trailers. It does not rewrite commit messages or silently remove attribution.

Git hooks are local safeguards and can be bypassed by changing local Git configuration or explicitly disabling hooks. The CI workflow independently checks commit messages on pushes and pull requests. Branch protection must require the attribution check for the policy to be mandatory before merging.
