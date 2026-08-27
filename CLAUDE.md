# CLAUDE.md

This project's agent guidance lives in **[AGENTS.md](./AGENTS.md)** — read it first.

This is a Greentic Designer `design` extension (`greentic.rag-qdrant` v0.2.0) scaffolded
by `gtdx new`, exposing six RAG tools over a Qdrant collection (`rag_search`, `rag_upsert`,
`rag_ingest`, `rag_delete`, `rag_collection_ensure`, `rag_list`). AGENTS.md covers the
build/publish workflow (`gtdx dev`, `gtdx publish`, `./ci/local_check.sh`), the module
layout — `lib.rs` is the only file allowed to touch WIT bindings, everything else is pure
and host-testable — and which files are generated and must never be hand-edited (the
`describe.json` `sha256` fields, `.gtdx-contract.lock`, `wit/deps/`, and `src/bindings.rs`).
