# Contributing to Celetixo

## Commit attribution

Git commit authorship and co-authorship must represent human contributors.

- Do not add AI tools, coding agents, automated assistants, or bots as Git authors or co-authors.
- Do not add `Co-authored-by` trailers that identify an AI system, agent, bot, or automated service.
- Human co-authors may be credited with their consent and their actual Git identity.
- AI-assisted implementation does not require a Git co-author trailer.
- This policy concerns Git attribution metadata. It does not prohibit accurate disclosure of AI assistance in engineering records where appropriate.

### Local enforcement

Celetixo provides a repository-managed `commit-msg` hook in `.githooks/`. Configure Git to use it from the repository root:

```sh
git config --local core.hooksPath .githooks
```

The hook rejects recognized automated identities in `Co-authored-by` trailers. It does not rewrite commit messages or silently remove attribution. If a legitimate human identity is incorrectly flagged, review the hook's deny patterns and update them deliberately.

Git hooks are local safeguards and can be bypassed by changing local Git configuration or explicitly disabling hooks. Contributors must keep the configured hook enabled. Repository automation should independently enforce this policy where commit metadata is available.
