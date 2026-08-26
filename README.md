# component-rag-qdrant-ext

A Greentic Designer **design** extension exposing five RAG tools — over both
flow nodes and the agentic worker — backed by a Qdrant vector collection:

- `rag_search` — semantic search by query text or a pre-computed vector.
- `rag_upsert` — store or replace a single point by id.
- `rag_ingest` — chunk, embed and store a whole document under a `doc_id`;
  re-ingesting the same `doc_id` replaces its previous chunks so nothing is
  duplicated or orphaned.
- `rag_delete` — delete points by id, or every chunk of one `doc_id`.
- `rag_collection_ensure` — create the collection if it does not exist yet.

Text is embedded through a configurable OpenAI-shaped embeddings API; callers
that already hold a vector can pass it directly instead.

- id: `greentic.rag-qdrant`
- version: `0.1.0`
- contract: `greentic:extension-design@0.3.0`

## Configuration

Passed to `lifecycle::init` as JSON:

| Field | Required | Default | Notes |
|---|---|---|---|
| `qdrant_url` | yes | — | e.g. `https://xyz.qdrant.io:6333`. Trailing slash is stripped. |
| `collection` | yes | — | Default collection; every tool can override it per call. |
| `embedding.base_url` | yes | — | `/embeddings` is appended. Any OpenAI-shaped API. |
| `embedding.model` | yes | — | e.g. `text-embedding-3-small`. |
| `embedding.dimensions` | yes | — | Must match the collection's vector width. |
| `chunk.max_chars` | no | `1200` | Characters, not bytes. |
| `chunk.overlap_chars` | no | `150` | Must be less than `max_chars`. |

## Secrets

`rag-qdrant/qdrant_api_key` and `rag-qdrant/embedding_api_key`.

## Requirements and limits

- **Qdrant 1.10 or newer.** Search uses the Query API (`/points/query`); the
  legacy `/points/search` path is not used.
- **`runtime.permissions.network` in `describe.json` grants `https://*.qdrant.io/*`.**
  Qdrant **Cloud** works out of the box — that wildcard covers every tenant's
  cluster host, no editing required. **Self-hosted** Qdrant is not covered:
  its host must be added to the allowlist by hand. Likewise, if
  `embedding.base_url` is configured to anything other than
  `https://api.openai.com`, that host must be added to the same allowlist too,
  or the embeddings call is rejected before it ever reaches the network.
  (The wildcard is verified against the SDK's host mock, which matches on
  host only; if a runtime instead matches on the full URL including port, a
  `:6333` variant of the pattern may be needed.)
- **Ingestion takes text, not files.** The host exposes no filesystem, so PDF
  and DOCX must be converted to text before calling `rag_ingest`.
- **Designer 1.2.0 or newer** (`compat.min_designer_version`). The published
  designer at 0.6.0 cannot run this extension.

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
- `src/ops.rs`    — Qdrant/embeddings HTTP calls and secret handling
- `src/tool_meta.rs` — static tool metadata (schemas, capabilities, agentic-worker hints); also generates `describe.json`'s `contributions.tools` block
- `wit/`          — WIT contract (vendored by `gtdx new`; see `.gtdx-contract.lock`)
- `i18n/en.json`  — user-facing strings
- `AGENTS.md`     — guidance for AI coding agents (Claude Code, Codex, …)
- `CLAUDE.md`     — Claude Code entry point (points to `AGENTS.md`)
- `.claude/`      — Claude Code config: pre-approved build perms + `/check` command
