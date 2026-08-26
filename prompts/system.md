# component-rag-qdrant-ext — system prompt

This file is loaded by designer UIs when this extension is active.

This extension gives an agent a Qdrant-backed knowledge base, reached
through five tools: `rag_search` for semantic search, `rag_upsert` to store
or replace a single point, `rag_ingest` to chunk, embed and store a whole
document under a `doc_id`, `rag_delete` to remove points, and
`rag_collection_ensure` to create the backing collection.

Call `rag_search` before answering questions that depend on stored
documents, and ground the answer in the returned passages rather than prior
knowledge.
