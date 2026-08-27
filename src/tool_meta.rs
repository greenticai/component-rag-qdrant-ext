//! Static metadata for the five RAG tools. Pure, so the capability flags and
//! the agentic-worker metadata are asserted by a host test rather than
//! discovered in production.

pub const SEARCH_TOOL: &str = "rag_search";
pub const UPSERT_TOOL: &str = "rag_upsert";
pub const INGEST_TOOL: &str = "rag_ingest";
pub const DELETE_TOOL: &str = "rag_delete";
pub const ENSURE_TOOL: &str = "rag_collection_ensure";
pub const LIST_TOOL: &str = "rag_list";

pub struct ToolMeta {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema_json: &'static str,
    pub output_schema_json: &'static str,
    pub capabilities: Vec<String>,
    pub agentic_worker_metadata: &'static str,
}

const HITS_OUTPUT: &str = r#"{
  "type": "object",
  "required": ["hits"],
  "properties": {
    "hits": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "id": { "type": "string" },
          "score": { "type": "number" },
          "payload": { "type": "object" }
        }
      }
    }
  }
}"#;

const ACK_OUTPUT: &str = r#"{
  "type": "object",
  "required": ["ok"],
  "properties": { "ok": { "type": "boolean" } }
}"#;

const SEARCH_INPUT: &str = r#"{
  "type": "object",
  "properties": {
    "query": { "type": "string", "description": "Text to search for. Pass this OR vector, not both." },
    "vector": { "type": "array", "items": { "type": "number" }, "description": "A pre-computed embedding. Pass this OR query, not both." },
    "top_k": { "type": "integer", "minimum": 1, "description": "How many results to return (default 5)." },
    "filter": { "type": "object", "description": "A Qdrant filter object, passed through verbatim." },
    "collection": { "type": "string", "description": "Override the configured default collection." }
  },
  "oneOf": [
    { "required": ["query"],  "not": { "required": ["vector"] } },
    { "required": ["vector"], "not": { "required": ["query"] } }
  ]
}"#;

const UPSERT_INPUT: &str = r#"{
  "type": "object",
  "required": ["id"],
  "properties": {
    "id": { "type": "string", "description": "Point id. Must be an unsigned integer or a UUID." },
    "text": { "type": "string", "description": "Text to embed and store. Pass this OR vector, not both." },
    "vector": { "type": "array", "items": { "type": "number" }, "description": "A pre-computed embedding. Pass this OR text, not both." },
    "payload": { "type": "object", "description": "Arbitrary metadata stored alongside the point." },
    "collection": { "type": "string", "description": "Override the configured default collection." }
  },
  "oneOf": [
    { "required": ["text"],   "not": { "required": ["vector"] } },
    { "required": ["vector"], "not": { "required": ["text"] } }
  ]
}"#;

const INGEST_INPUT: &str = r#"{
  "type": "object",
  "required": ["doc_id", "text"],
  "properties": {
    "doc_id": { "type": "string", "description": "Stable document identifier. Re-ingesting the same doc_id replaces its chunks." },
    "text": { "type": "string", "description": "Full document text. It is chunked, embedded and stored." },
    "metadata": { "type": "object", "description": "Metadata copied onto every chunk of this document." },
    "collection": { "type": "string", "description": "Override the configured default collection." }
  }
}"#;

const DELETE_INPUT: &str = r#"{
  "type": "object",
  "properties": {
    "ids": { "type": "array", "items": { "type": "string" }, "description": "Point ids to delete. Pass this OR doc_id, not both." },
    "doc_id": { "type": "string", "description": "Delete every chunk of this document. Pass this OR ids, not both." },
    "collection": { "type": "string", "description": "Override the configured default collection." }
  },
  "oneOf": [
    { "required": ["ids"],    "not": { "required": ["doc_id"] } },
    { "required": ["doc_id"], "not": { "required": ["ids"] } }
  ]
}"#;

const ENSURE_INPUT: &str = r#"{
  "type": "object",
  "properties": {
    "collection": { "type": "string", "description": "Collection to create. Defaults to the configured one." },
    "dimensions": { "type": "integer", "minimum": 1, "description": "Vector width. Defaults to the configured embedding dimensions." },
    "distance": { "type": "string", "enum": ["Cosine", "Dot", "Euclid"], "description": "Distance metric (default Cosine)." }
  }
}"#;

const LIST_INPUT: &str = r#"{
  "type": "object",
  "properties": {
    "limit": { "type": "integer", "minimum": 1, "description": "Max chunks to scan per page (default 50). Chunk counts in the response are only for chunks returned within this page." },
    "offset": { "description": "Opaque page cursor from a previous call's next_page_offset. Omit to start from the first page." },
    "filter": { "type": "object", "description": "A Qdrant filter object, passed through verbatim." },
    "collection": { "type": "string", "description": "Override the configured default collection." }
  }
}"#;

const LIST_OUTPUT: &str = r#"{
  "type": "object",
  "required": ["documents"],
  "properties": {
    "documents": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "doc_id": { "type": "string" },
          "chunk_count": { "type": "integer" },
          "metadata": { "type": "object" }
        }
      }
    },
    "next_page_offset": { "description": "Present only when another page exists. Pass verbatim as `offset` on the next call." }
  }
}"#;

const SEARCH_META: &str = r#"{
  "usage_hint": "Retrieve passages from the knowledge base by meaning. Pass a natural-language query; the extension embeds it and returns the closest stored chunks with their scores and metadata. Use this before answering anything that depends on stored documents.",
  "side_effects": "read",
  "cost": "low",
  "confirmation_required": false
}"#;

const UPSERT_META: &str = r#"{
  "usage_hint": "Store or replace one point by id. Use for short, self-contained facts where you control the id. For a whole document, use rag_ingest instead so it gets chunked.",
  "side_effects": "write",
  "cost": "low",
  "confirmation_required": false
}"#;

const INGEST_META: &str = r#"{
  "usage_hint": "Upload a whole document: it is chunked, embedded, and stored under doc_id. Re-ingesting the same doc_id replaces all of that document's existing chunks.",
  "side_effects": "write",
  "cost": "medium",
  "confirmation_required": true
}"#;

const DELETE_META: &str = r#"{
  "usage_hint": "Remove points, either by explicit ids or by deleting every chunk of one doc_id. This is not recoverable.",
  "side_effects": "write",
  "cost": "low",
  "confirmation_required": true
}"#;

const ENSURE_META: &str = r#"{
  "usage_hint": "Create the collection if it does not exist, fixing its vector width and distance metric. Safe to call repeatedly; ingest and upsert already call it.",
  "side_effects": "write",
  "cost": "low",
  "confirmation_required": true
}"#;

const LIST_META: &str = r#"{
  "usage_hint": "Enumerate the documents stored in the knowledge base, grouped by doc_id with a chunk count and the metadata stored at ingest. Use this to answer what is in the knowledge base before deciding what to search, ingest or delete. Results are paginated: pass the previous response's next_page_offset back as offset to continue.",
  "side_effects": "read",
  "cost": "low",
  "confirmation_required": false
}"#;

fn both() -> Vec<String> {
    vec!["flow".to_string(), "agentic_worker".to_string()]
}

/// Every tool this extension contributes, in catalog order.
#[must_use]
pub fn all_tools() -> Vec<ToolMeta> {
    vec![
        ToolMeta {
            name: SEARCH_TOOL,
            description: "Semantic search over the Qdrant knowledge base.",
            input_schema_json: SEARCH_INPUT,
            output_schema_json: HITS_OUTPUT,
            capabilities: both(),
            agentic_worker_metadata: SEARCH_META,
        },
        ToolMeta {
            name: UPSERT_TOOL,
            description: "Store or replace a single point by id.",
            input_schema_json: UPSERT_INPUT,
            output_schema_json: ACK_OUTPUT,
            capabilities: both(),
            agentic_worker_metadata: UPSERT_META,
        },
        ToolMeta {
            name: INGEST_TOOL,
            description: "Chunk, embed and store a whole document under a doc_id.",
            input_schema_json: INGEST_INPUT,
            output_schema_json: ACK_OUTPUT,
            capabilities: both(),
            agentic_worker_metadata: INGEST_META,
        },
        ToolMeta {
            name: DELETE_TOOL,
            description: "Delete points by id, or every chunk of one document.",
            input_schema_json: DELETE_INPUT,
            output_schema_json: ACK_OUTPUT,
            capabilities: both(),
            agentic_worker_metadata: DELETE_META,
        },
        ToolMeta {
            name: ENSURE_TOOL,
            description: "Create the collection if absent, with a fixed vector width and metric.",
            input_schema_json: ENSURE_INPUT,
            output_schema_json: ACK_OUTPUT,
            capabilities: both(),
            agentic_worker_metadata: ENSURE_META,
        },
        ToolMeta {
            name: LIST_TOOL,
            description: "Enumerate stored documents, grouped by doc_id, with pagination.",
            input_schema_json: LIST_INPUT,
            output_schema_json: LIST_OUTPUT,
            capabilities: both(),
            agentic_worker_metadata: LIST_META,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_six_tools_are_listed() {
        let names: Vec<&str> = all_tools().into_iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            vec![
                "rag_search",
                "rag_upsert",
                "rag_ingest",
                "rag_delete",
                "rag_collection_ensure",
                "rag_list",
            ]
        );
    }

    #[test]
    fn every_tool_is_available_to_both_flow_and_agentic_worker() {
        for tool in all_tools() {
            assert!(
                tool.capabilities.contains(&"flow".to_string())
                    && tool.capabilities.contains(&"agentic_worker".to_string()),
                "{} is missing a capability: {:?}",
                tool.name,
                tool.capabilities
            );
        }
    }

    #[test]
    fn every_schema_is_valid_json() {
        for tool in all_tools() {
            serde_json::from_str::<serde_json::Value>(tool.input_schema_json)
                .unwrap_or_else(|e| panic!("{} input schema: {e}", tool.name));
            serde_json::from_str::<serde_json::Value>(tool.output_schema_json)
                .unwrap_or_else(|e| panic!("{} output schema: {e}", tool.name));
        }
    }

    /// A tool that ships no metadata is treated by the runtime as
    /// `side_effects: external` + `confirmation_required: true`, which would
    /// make every search prompt the user.
    #[test]
    fn every_tool_declares_side_effects_and_a_confirmation_stance() {
        for tool in all_tools() {
            let meta: serde_json::Value = serde_json::from_str(tool.agentic_worker_metadata)
                .unwrap_or_else(|e| panic!("{} metadata: {e}", tool.name));
            assert!(
                meta.get("side_effects").is_some(),
                "{} has no side_effects",
                tool.name
            );
            assert!(
                meta.get("confirmation_required").is_some(),
                "{} has no confirmation_required",
                tool.name
            );
            assert!(
                meta.get("usage_hint").is_some(),
                "{} has no usage_hint",
                tool.name
            );
        }
    }

    /// `input.rs` rejects "both" and "neither" outright. A model that reads only
    /// `required`/`properties` and skips the prose would keep constructing those
    /// calls and keep getting InvalidInput, so the constraint is stated formally
    /// too — and asserted here, or it silently rots out of the schemas.
    #[test]
    fn the_either_or_tools_encode_that_constraint_formally_not_only_in_prose() {
        for (name, pair) in [
            ("rag_search", ["query", "vector"]),
            ("rag_upsert", ["text", "vector"]),
            ("rag_delete", ["ids", "doc_id"]),
        ] {
            let tool = all_tools()
                .into_iter()
                .find(|t| t.name == name)
                .unwrap_or_else(|| panic!("{name} missing"));
            let schema: serde_json::Value = serde_json::from_str(tool.input_schema_json).unwrap();
            let branches = schema["oneOf"]
                .as_array()
                .unwrap_or_else(|| panic!("{name} has no oneOf"))
                .clone();
            assert_eq!(branches.len(), 2, "{name}");
            for (i, branch) in branches.iter().enumerate() {
                assert_eq!(branch["required"][0], pair[i], "{name} branch {i}");
                assert_eq!(
                    branch["not"]["required"][0],
                    pair[1 - i],
                    "{name} branch {i}"
                );
            }
        }
    }

    #[test]
    fn reads_do_not_ask_for_confirmation_and_writes_do() {
        for tool in all_tools() {
            let meta: serde_json::Value =
                serde_json::from_str(tool.agentic_worker_metadata).unwrap();
            let confirm = meta["confirmation_required"].as_bool().unwrap();
            match tool.name {
                "rag_search" => assert!(!confirm, "search must not prompt"),
                "rag_upsert" => assert!(!confirm, "upsert must not prompt"),
                "rag_list" => assert!(!confirm, "list must not prompt"),
                _ => assert!(confirm, "{} must prompt", tool.name),
            }
        }
    }

    /// `describe.json`'s `contributions.tools` entries are generated from this
    /// file by `print_contributions` below, but nothing stops someone from
    /// editing a schema here without re-running the generator — `cargo test`,
    /// clippy, `gtdx validate` and `gtdx lint` all stay green while the
    /// designer catalogues the stale copy. This guards against that drift.
    #[test]
    fn describe_json_matches_the_tool_metadata_in_this_file() {
        let manifest: serde_json::Value = serde_json::from_str(include_str!("../describe.json"))
            .expect("describe.json must be valid JSON");
        let published = manifest["contributions"]["tools"]
            .as_array()
            .expect("describe.json has no contributions.tools array");

        let tools = all_tools();
        assert_eq!(
            published.len(),
            tools.len(),
            "describe.json has {} contributed tools but tool_meta.rs declares {} — \
             a tool was added in one place and not the other",
            published.len(),
            tools.len()
        );

        for tool in &tools {
            let entry = published
                .iter()
                .find(|e| e["name"] == tool.name)
                .unwrap_or_else(|| {
                    panic!(
                        "describe.json has no contributions.tools entry named {}",
                        tool.name
                    )
                });

            assert_eq!(
                entry["name"], tool.name,
                "{}: name drifted from describe.json",
                tool.name
            );
            assert_eq!(
                entry["description"], tool.description,
                "{}: description drifted from describe.json",
                tool.name
            );

            // Compared as parsed JSON, not as strings, so formatting
            // differences (whitespace, key order) cannot cause a false
            // failure — only an actual schema difference should.
            let published_schema_str = entry["input_schema"].as_str().unwrap_or_else(|| {
                panic!("{}: describe.json input_schema is not a string", tool.name)
            });
            let published_schema: serde_json::Value = serde_json::from_str(published_schema_str)
                .unwrap_or_else(|e| {
                    panic!(
                        "{}: describe.json input_schema is not valid JSON: {e}",
                        tool.name
                    )
                });
            let code_schema: serde_json::Value = serde_json::from_str(tool.input_schema_json)
                .unwrap_or_else(|e| {
                    panic!(
                        "{}: tool_meta.rs input schema is not valid JSON: {e}",
                        tool.name
                    )
                });
            assert_eq!(
                published_schema, code_schema,
                "{}: input_schema in describe.json has drifted from tool_meta.rs — \
                 regenerate with `RUNTIME_REF=rag-qdrant cargo test print_contributions -- --ignored --nocapture`",
                tool.name
            );
        }
    }

    /// Not an assertion — a generator. Prints the `contributions.tools` block
    /// for describe.json so the schemas are never hand-copied out of sync.
    /// Run: `cargo test print_contributions -- --ignored --nocapture`
    #[test]
    #[ignore = "generator, not a check"]
    fn print_contributions() {
        let runtime_ref = std::env::var("RUNTIME_REF").unwrap_or_default();
        let entries: Vec<serde_json::Value> = all_tools()
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "export": "greentic:extension-design/tools.invoke-tool",
                    "runtime_ref": runtime_ref,
                    "capabilities": t.capabilities,
                    "description": t.description,
                    "input_schema": t.input_schema_json,
                    "secret_requirements": [
                        {"key": "rag-qdrant/qdrant_api_key", "required": true, "format": "text"},
                        {"key": "rag-qdrant/embedding_api_key", "required": true, "format": "text"}
                    ]
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "tools": entries })).unwrap()
        );
    }
}
