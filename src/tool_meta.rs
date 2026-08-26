//! Static metadata for the five RAG tools. Pure, so the capability flags and
//! the agentic-worker metadata are asserted by a host test rather than
//! discovered in production.

pub const SEARCH_TOOL: &str = "rag_search";
pub const UPSERT_TOOL: &str = "rag_upsert";
pub const INGEST_TOOL: &str = "rag_ingest";
pub const DELETE_TOOL: &str = "rag_delete";
pub const ENSURE_TOOL: &str = "rag_collection_ensure";

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
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_five_tools_are_listed() {
        let names: Vec<&str> = all_tools().into_iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            vec![
                "rag_search",
                "rag_upsert",
                "rag_ingest",
                "rag_delete",
                "rag_collection_ensure"
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
            let meta: serde_json::Value =
                serde_json::from_str(tool.agentic_worker_metadata)
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
            assert!(meta.get("usage_hint").is_some(), "{} has no usage_hint", tool.name);
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
            let schema: serde_json::Value =
                serde_json::from_str(tool.input_schema_json).unwrap();
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
                _ => assert!(confirm, "{} must prompt", tool.name),
            }
        }
    }
}
