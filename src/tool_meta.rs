//! Static metadata for the six RAG tools. Pure, so the capability flags and
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
    "collection": { "type": "string", "description": "Override the configured default collection. Rejected when the host supplies a per-tenant collection for the caller \u2014 in a multi-tenant install the collection is chosen by tenant configuration, not per call." }
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
    "collection": { "type": "string", "description": "Override the configured default collection. Rejected when the host supplies a per-tenant collection for the caller \u2014 in a multi-tenant install the collection is chosen by tenant configuration, not per call." }
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
    "collection": { "type": "string", "description": "Override the configured default collection. Rejected when the host supplies a per-tenant collection for the caller \u2014 in a multi-tenant install the collection is chosen by tenant configuration, not per call." }
  }
}"#;

const DELETE_INPUT: &str = r#"{
  "type": "object",
  "properties": {
    "ids": { "type": "array", "items": { "type": "string" }, "description": "Point ids to delete. Pass this OR doc_id, not both." },
    "doc_id": { "type": "string", "description": "Delete every chunk of this document. Pass this OR ids, not both." },
    "collection": { "type": "string", "description": "Override the configured default collection. Rejected when the host supplies a per-tenant collection for the caller \u2014 in a multi-tenant install the collection is chosen by tenant configuration, not per call." }
  },
  "oneOf": [
    { "required": ["ids"],    "not": { "required": ["doc_id"] } },
    { "required": ["doc_id"], "not": { "required": ["ids"] } }
  ]
}"#;

const ENSURE_INPUT: &str = r#"{
  "type": "object",
  "properties": {
    "collection": { "type": "string", "description": "Collection to create. Defaults to the configured one. Rejected when the host supplies a per-tenant collection for the caller." },
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
    "collection": { "type": "string", "description": "Override the configured default collection. Rejected when the host supplies a per-tenant collection for the caller \u2014 in a multi-tenant install the collection is chosen by tenant configuration, not per call." }
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
  "usage_hint": "Enumerate the documents stored in the knowledge base, grouped by doc_id with a chunk count and the metadata stored at ingest. Use this to answer what is in the knowledge base before deciding what to search, ingest or delete. Results are paginated: pass the previous response's next_page_offset back as offset to continue. chunk_count reflects only the current page, not the document's total chunk count — a document whose chunks straddle a page boundary will show a partial count on each page it appears in.",
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

    /// `describe.json`'s top-level `configSchema` is a *string* holding a
    /// JSON Schema — the describe is data, and nothing else in this suite
    /// touches it, so a typo (unbalanced quotes, a dropped field) would
    /// otherwise ship unnoticed while `cargo test`, clippy and `gtdx lint`
    /// all stayed green. `config.rs::resolve` treats `qdrant_url` and
    /// `collection` as the only two fields with no working default (see its
    /// module docs), so this also pins that the form the admin console
    /// renders actually asks for both of them.
    #[test]
    fn describe_json_config_schema_parses_and_names_the_required_fields() {
        let manifest: serde_json::Value = serde_json::from_str(include_str!("../describe.json"))
            .expect("describe.json must be valid JSON");
        let config_schema_str = manifest["configSchema"]
            .as_str()
            .expect("describe.json must have a top-level configSchema string");

        let config_schema: serde_json::Value =
            serde_json::from_str(config_schema_str).expect("configSchema must parse as JSON");
        assert!(
            config_schema.is_object(),
            "configSchema must parse to a JSON object, not {config_schema}"
        );

        let properties = config_schema["properties"]
            .as_object()
            .expect("configSchema must declare properties");
        assert!(
            properties.contains_key("qdrant_url"),
            "configSchema must name qdrant_url — config.rs::resolve treats it as required"
        );
        assert!(
            properties.contains_key("collection"),
            "configSchema must name collection — config.rs::resolve treats it as required"
        );

        let required = config_schema["required"]
            .as_array()
            .expect("configSchema must declare a required array")
            .iter()
            .map(|v| v.as_str().unwrap_or_default())
            .collect::<Vec<_>>();
        assert!(
            required.contains(&"qdrant_url"),
            "qdrant_url has no working default and must be required"
        );
        assert!(
            required.contains(&"collection"),
            "collection has no working default and must be required"
        );
    }

    /// The tools that would become flow palette entries, if this extension
    /// could contribute any — see
    /// `node_types_must_resolve_to_a_published_flow_component` for why it
    /// currently cannot.
    ///
    /// Deliberately a subset of the six. A node type is something a flow
    /// author drags onto a canvas and the runner then executes unattended:
    /// no confirmation step, no human in the loop. That is a higher bar than
    /// the agentic worker (which gates on `confirmation_required`, asserted
    /// in `reads_do_not_ask_for_confirmation_and_writes_do`) or the Knowledge
    /// base view (which confirms in-page). Kept:
    ///
    /// - `rag_search` — the reason a flow reaches for this extension at all:
    ///   retrieve grounding passages before a step that answers. Read-only.
    /// - `rag_ingest` — the other half, populating the knowledge base from a
    ///   pipeline. Idempotent by `doc_id`: `ops::ingest` deletes that
    ///   document's chunks and rewrites them, so a re-run replaces rather
    ///   than duplicates.
    ///
    /// Left out, and why:
    ///
    /// - `rag_upsert` — the low-level twin of `rag_ingest`. It demands a
    ///   Qdrant-legal point id (unsigned integer or UUID), stores text
    ///   unchunked, and a repeat id silently replaces that point's entire
    ///   payload with no merge and no read-before-write. `rag_ingest` serves
    ///   the same intent with any string `doc_id`. Two write entries where
    ///   one fails on an id typo is a worse palette than one that works.
    /// - `rag_delete` — irreversible, and unbounded when given a `doc_id`:
    ///   `ops::delete` returns a bare `{"ok": true}` whether it removed zero
    ///   points or ten thousand, so a mistyped id is indistinguishable from a
    ///   correct deletion. The two surfaces that do expose it both ask first.
    ///   A flow node would not.
    /// - `rag_collection_ensure` — one-time deployment setup, and
    ///   `ops::ingest` / `ops::upsert` already ensure the collection on every
    ///   write. A flow never needs it.
    /// - `rag_list` — inventory rather than automation: `limit` counts chunks
    ///   and not documents, `chunk_count` covers only the page in hand, and
    ///   consuming it means a cursor loop. The Knowledge base view already
    ///   does this job with a UI built for it.
    ///
    /// All six remain available to the agentic worker and to the view.
    /// `contributions.tools[].capabilities` is a dispatch gate, not a palette
    /// registration, so nothing here withdraws a tool from anything.
    #[cfg(test)]
    const INTENDED_FLOW_PALETTE: &[(&str, &str)] = &[
        ("rag-qdrant.search", SEARCH_TOOL),
        ("rag-qdrant.ingest", INGEST_TOOL),
    ];

    /// A node type only reaches a running flow if its `runtime_ref` names a
    /// component carrying an `oci_ref`. This extension has no such component,
    /// so it contributes none — and this test is what stops one being added
    /// back without one.
    ///
    /// The trap is that every gate an author would think to run stays green.
    /// `gtdx validate` accepts it (`describe-v2.json` declares
    /// `nodeTypes: {"type": "array", "items": {}}` — no sub-schema at all),
    /// and `gtdx lint` has exactly one rule that reads node types,
    /// `E_RUNTIME_REF`, which only checks that the ref names *some* key in
    /// `runtime.components`. Pointing it at this crate's own design-extension
    /// wasm satisfies that and lints clean at both `gtdx lint` and
    /// `gtdx lint --publish`. What happens next is silent:
    ///
    /// - greentic-designer's `orchestrate::pack_via_packc::ext_nodes` indexes
    ///   a node type only when `runtime.components.<runtime_ref>.oci_ref` is
    ///   present. This crate ships a `gtpack`, not an `oci_ref`, so the entry
    ///   is dropped from the index — and `graph::node_kind` is TOTAL, so the
    ///   unrecognised canvas node falls through to `"adaptive-card"` and is
    ///   packed as a blank AdaptiveCard. The component is never vendored and
    ///   the `operation` never runs, in Run Demo, in `/api/pack`, and so in
    ///   any deployed bundle.
    /// - `flow_generator::compiler::resolve` says it outright: "If there is
    ///   no oci_ref (gtpack-only), fall through to catalog pin" — and
    ///   `flow_generator/catalog.baseline.yaml` pins nothing for these ids.
    /// - greentic-runner-host accepts a component as a node runtime only if
    ///   it exports `node@0.5` / `node@0.4` / `component-runtime@0.6`. A
    ///   design extension exports `greentic:extension-design/tools`, so it
    ///   could not execute the node even if it were reachable. This is the
    ///   bug the SDK removed from its own `wasm-component` scaffold in #106.
    ///
    /// Meanwhile `catalog_dynamic::capability_from_node_type` is pure and
    /// total and builds the palette entry regardless. So the entry appears,
    /// an author uses it, and the pack silently contains a blank card.
    ///
    /// Unblocking this needs a separate flow component — world
    /// `greentic:component/component-v0-v6-v0@0.6.0`, a crate in
    /// `greenticai/components-public`, published to GHCR and pinned here by
    /// digest — which is how `greentic.tavily` does it. Once that exists, add
    /// it to `runtime.components` beside `rag-qdrant`, declare the entries in
    /// `INTENDED_FLOW_PALETTE` above, and this test checks the wiring.
    ///
    /// # What a palette tile is missing is discoverability, not reach
    ///
    /// These tools are already callable from a running flow, without any node
    /// type, because all six declare `agentic_worker`. A flow's `dw.agent` /
    /// `dw.agent_graph` node dispatches to them through
    /// `greentic-aw-runtime::tools::dispatch_tool_call`, which falls through
    /// to `ExtensionRuntime::invoke_tool_ctx` — the same
    /// `greentic:extension-design/tools.invoke-tool` export the view bridge
    /// uses. `rag_search` in particular has a purpose-built binding: a
    /// `provider.knowledge.extension` knowledge provider names this extension
    /// in `provider.knowledge.extension.extension_id` and the retrieval tool
    /// in `provider.knowledge.extension.tool_name`. That is the supported way
    /// to ground an agent on this knowledge base today, and it is why the
    /// absence of node types is a gap in discoverability rather than in
    /// capability.
    #[test]
    fn node_types_must_resolve_to_a_published_flow_component() {
        let manifest: serde_json::Value = serde_json::from_str(include_str!("../describe.json"))
            .expect("describe.json must be valid JSON");

        let nodes = match manifest["contributions"]["nodeTypes"].as_array() {
            Some(nodes) => nodes,
            // No node types is the current, deliberate state. The doc comment
            // above records why; there is nothing to check.
            None => return,
        };

        let components = manifest["runtime"]["components"]
            .as_object()
            .expect("describe.json has no runtime.components object");
        let published_tools = manifest["contributions"]["tools"]
            .as_array()
            .expect("describe.json has no contributions.tools array");
        let tool_names: Vec<&str> = all_tools().into_iter().map(|t| t.name).collect();

        let declared: Vec<(&str, &str)> = nodes
            .iter()
            .map(|n| {
                let type_id = n["type_id"]
                    .as_str()
                    .expect("node type_id must be a string");
                let operation = n["operation"].as_str().unwrap_or_else(|| {
                    panic!(
                        "{type_id} sets no operation. The runner REQUIRES it and does not \
                         default — it refuses the node with \"expected \
                         node.component.operation to be set\", at execution time, after \
                         the palette and the pack build have both reported success."
                    )
                });
                (type_id, operation)
            })
            .collect();

        assert_eq!(
            declared,
            INTENDED_FLOW_PALETTE.to_vec(),
            "the flow palette in describe.json has drifted from \
             INTENDED_FLOW_PALETTE — which tools become nodes is a deliberate \
             decision, so change both together and record the reasoning in that \
             constant's doc comment"
        );

        for node in nodes {
            let type_id = node["type_id"].as_str().unwrap_or_default();
            let operation = node["operation"].as_str().unwrap_or_default();

            assert!(
                tool_names.contains(&operation),
                "{type_id} runs operation {operation:?}, which is not a tool this \
                 extension contributes (have: {tool_names:?}) — the palette entry \
                 would fail when someone ran the flow"
            );

            // Deliberately NOT asserted here: that the tool declares the
            // "flow" capability. `capabilities` is a different axis, and the
            // name is misleading — the designer's `tool_bridge::defs::
            // is_chat_surface` reads "flow" to decide whether a tool joins the
            // DESIGNER CHAT assistant's tool-calling loop, while the runner's
            // `manifest_tools` filters on "agentic_worker" to decide what a
            // running flow's `dw.agent` node may call. Neither has any bearing
            // on the palette, which is `nodeTypes` alone.
            assert!(
                published_tools.iter().any(|t| t["name"] == operation),
                "{type_id} runs {operation:?}, absent from contributions.tools"
            );

            let runtime_ref = node["runtime_ref"].as_str().unwrap_or_else(|| {
                panic!(
                    "{type_id} has no runtime_ref. Absent does NOT mean \"use the only \
                     declared component\" for a node type — that is the rule for tools. \
                     The designer falls through to the pin in \
                     flow_generator/catalog.baseline.yaml, which knows nothing about \
                     this extension, so the node resolves to nothing."
                )
            });
            let component = components.get(runtime_ref).unwrap_or_else(|| {
                panic!(
                    "{type_id}'s runtime_ref {runtime_ref:?} names no key in \
                     runtime.components (have: {:?}) — gtdx lint reports this one as \
                     E_RUNTIME_REF",
                    components.keys().collect::<Vec<_>>()
                )
            });

            assert!(
                component["oci_ref"].as_str().is_some_and(|r| !r.is_empty()),
                "{type_id} points at {runtime_ref:?}, which has no oci_ref. The \
                 designer's pack build indexes a node type only when \
                 runtime.components.<runtime_ref>.oci_ref is present; without it the \
                 canvas node falls through node_kind's total match and is packed as a \
                 blank AdaptiveCard. gtdx lint does not catch this: {component}"
            );
            assert!(
                component["gtpack"].is_null(),
                "{type_id}'s node component must not also be an in-pack gtpack — the \
                 design-extension wasm cannot execute a node, it exports \
                 greentic:extension-design/tools rather than component-runtime: \
                 {component}"
            );
            assert!(
                component["oci_ref"]
                    .as_str()
                    .is_some_and(|r| r.contains("@sha256:")),
                "{type_id}'s component must be pinned by digest, not by tag — a built \
                 pack embeds the reference permanently: {component}"
            );

            assert!(
                !node["output_ports"]
                    .as_array()
                    .unwrap_or(&Vec::new())
                    .is_empty(),
                "{type_id} has no output ports, so nothing can follow it in a flow"
            );

            let config_schema: serde_json::Value = node["config_schema"]
                .as_str()
                .map(|s| {
                    serde_json::from_str(s)
                        .unwrap_or_else(|e| panic!("{type_id}: config_schema is not JSON: {e}"))
                })
                .unwrap_or_else(|| panic!("{type_id}: config_schema must be a JSON string"));

            // NodeType is deny_unknown_fields and has no description field of
            // its own, so the operator-facing wording has nowhere to live but
            // the top of config_schema.
            assert!(
                config_schema["title"].is_string() && config_schema["description"].is_string(),
                "{type_id}: no operator-facing title/description. NodeType has no \
                 description field — the contract struct is deny_unknown_fields — so \
                 both belong at the top of config_schema."
            );

            let tool_meta = all_tools()
                .into_iter()
                .find(|t| t.name == operation)
                .unwrap_or_else(|| panic!("{type_id}: no tool named {operation}"));
            let input_schema: serde_json::Value =
                serde_json::from_str(tool_meta.input_schema_json).expect("tool input schema");
            let accepted = input_schema["properties"]
                .as_object()
                .unwrap_or_else(|| panic!("{operation} input schema declares no properties"));
            let offered = config_schema["properties"]
                .as_object()
                .unwrap_or_else(|| panic!("{type_id}: config_schema declares no properties"));

            for field in offered.keys() {
                assert!(
                    accepted.contains_key(field),
                    "{type_id}: the node form collects {field:?}, which {operation} does \
                     not accept — input.rs would reject the call at runtime"
                );
            }
            for required in input_schema["required"].as_array().unwrap_or(&Vec::new()) {
                let field = required.as_str().unwrap_or_default();
                assert!(
                    offered.contains_key(field),
                    "{type_id}: {operation} requires {field:?}, but the node form never \
                     collects it"
                );
            }
            assert!(
                !offered.contains_key("collection"),
                "{type_id}: the node form must not offer `collection`. The host stamps \
                 the tenant's collection via _tenant_overlay and ops::collection_of \
                 refuses a per-call override, so a flow author who filled this in would \
                 get an error on every run — the same rule the knowledge view follows."
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
