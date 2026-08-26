# component-rag-qdrant-ext

A Greentic Designer **design** extension.

- id: `greentic.rag-qdrant`
- version: `0.1.0`
- contract: `greentic:extension-design@0.3.0`

## Develop

```
gtdx dev           # watch, rebuild, and reinstall to local registry on save
```

## Publish

```
gtdx publish       # produce dist/greentic.rag-qdrant-0.1.0.gtxpack + install to local registry
```

## Layout

- `describe.json` — extension manifest
- `src/lib.rs`    — WASM guest exports
- `wit/`          — WIT contract (vendored by `gtdx new`; see `.gtdx-contract.lock`)
- `i18n/en.json`  — user-facing strings
- `AGENTS.md`     — guidance for AI coding agents (Claude Code, Codex, …)
- `CLAUDE.md`     — Claude Code entry point (points to `AGENTS.md`)
- `.claude/`      — Claude Code config: pre-approved build perms + `/check` command
