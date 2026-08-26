//! Greentic RAG/Qdrant design extension — WIT export layer.
//!
//! Exports five tools (`rag_search`, `rag_upsert`, `rag_ingest`, `rag_delete`,
//! `rag_collection_ensure`) via `greentic:extension-design/tools`. Every piece of
//! logic lives in the pure modules below; this file is the only one that may
//! touch `bindings`, because a host `cargo test` that reaches a WIT import
//! aborts the process outright.
#![allow(clippy::used_underscore_items)]

#[allow(warnings)]
pub mod bindings;

pub mod chunk;
pub mod config;
pub mod embed;
pub mod error;
pub mod host;
pub mod input;
pub mod ops;
pub mod qdrant;
pub mod tool_meta;

use bindings::exports::greentic::extension_base::{lifecycle, manifest};
use bindings::exports::greentic::extension_design::{knowledge, prompting, tools, validation};
use bindings::greentic::extension_base::types;

use error::RagError;
use host::{HostCalls, HttpRequest, HttpResponse};

pub struct Component;

/// The one and only WIT-backed `HostCalls`. Everything else takes the trait.
pub struct WitHost;

impl HostCalls for WitHost {
    fn fetch(&self, req: &HttpRequest) -> Result<HttpResponse, String> {
        let headers: Vec<(String, String)> = req.headers.clone();
        let wire = bindings::greentic::extension_host::http::Request {
            method: req.method.clone(),
            url: req.url.clone(),
            headers,
            body: req.body.clone(),
        };
        let resp = bindings::greentic::extension_host::http::fetch(&wire)?;
        Ok(HttpResponse {
            status: resp.status,
            body: resp.body,
        })
    }

    fn secret(&self, uri: &str) -> Result<String, String> {
        bindings::greentic::extension_host::secrets::get(uri)
    }
}

/// Map the extension's error taxonomy onto the WIT one. Kept as a free function
/// (not `From`) so it is callable from the host tests below.
#[must_use]
pub fn map_error(err: RagError) -> types::ExtensionError {
    match err {
        RagError::InvalidInput(m) => types::ExtensionError::InvalidInput(m),
        RagError::PermissionDenied(m) => types::ExtensionError::PermissionDenied(m),
        RagError::NotFound(m) => types::ExtensionError::NotFound(m),
        RagError::SchemaInvalid(m) => types::ExtensionError::SchemaInvalid(m),
        RagError::Internal(m) => types::ExtensionError::Internal(m),
    }
}

// ===== base::manifest =====
impl manifest::Guest for Component {
    fn get_identity() -> types::ExtensionIdentity {
        types::ExtensionIdentity {
            id: "greentic.rag-qdrant".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            kind: types::Kind::Design,
        }
    }

    fn get_offered() -> Vec<types::CapabilityRef> {
        vec![types::CapabilityRef {
            id: "greentic:rag/qdrant".to_string(),
            version: "1.0.0".to_string(),
        }]
    }

    fn get_required() -> Vec<types::CapabilityRef> {
        Vec::new()
    }
}

// ===== base::lifecycle =====
impl lifecycle::Guest for Component {
    fn init(config_json: String) -> Result<(), types::ExtensionError> {
        let cfg = config::parse_config(&config_json).map_err(map_error)?;
        config::store(cfg).map_err(map_error)
    }

    fn shutdown() {
        // Stateless: no client, no connection pool, nothing to drain.
    }
}

// ===== design::tools =====
impl tools::Guest for Component {
    fn list_tools() -> Vec<tools::ToolDefinition> {
        tool_meta::all_tools()
            .into_iter()
            .map(|meta| tools::ToolDefinition {
                name: meta.name.to_string(),
                description: meta.description.to_string(),
                input_schema_json: meta.input_schema_json.to_string(),
                output_schema_json: Some(meta.output_schema_json.to_string()),
                capabilities: Some(meta.capabilities),
                agentic_worker_metadata: Some(meta.agentic_worker_metadata.to_string()),
            })
            .collect()
    }

    fn invoke_tool(name: String, args_json: String) -> Result<String, types::ExtensionError> {
        dispatch(&name, &args_json).map_err(map_error)
    }
}

/// Parse arguments, then run the operation. Argument parsing happens before the
/// config lookup so a malformed call fails the same way whether or not `init`
/// has run.
fn dispatch(name: &str, args_json: &str) -> Result<String, RagError> {
    let host = WitHost;
    let value = match name {
        tool_meta::SEARCH_TOOL => {
            let input = input::parse_search(args_json)?;
            ops::search(&host, config::current()?, &input)?
        }
        tool_meta::UPSERT_TOOL => {
            let input = input::parse_upsert(args_json)?;
            ops::upsert(&host, config::current()?, &input)?
        }
        tool_meta::INGEST_TOOL => {
            let input = input::parse_ingest(args_json)?;
            ops::ingest(&host, config::current()?, &input)?
        }
        tool_meta::DELETE_TOOL => {
            let input = input::parse_delete(args_json)?;
            ops::delete(&host, config::current()?, &input)?
        }
        tool_meta::ENSURE_TOOL => {
            let input = input::parse_ensure(args_json)?;
            ops::ensure_collection(&host, config::current()?, &input)?
        }
        other => return Err(RagError::InvalidInput(format!("unknown tool: {other}"))),
    };
    serde_json::to_string(&value)
        .map_err(|e| RagError::Internal(format!("encode tool output: {e}")))
}

// ===== design::validation =====
impl validation::Guest for Component {
    fn validate_content(
        _content_type: String,
        _content_json: String,
    ) -> validation::ValidateResult {
        validation::ValidateResult {
            valid: true,
            diagnostics: Vec::new(),
        }
    }
}

// ===== design::prompting =====
impl prompting::Guest for Component {
    fn system_prompt_fragments() -> Vec<prompting::PromptFragment> {
        vec![prompting::PromptFragment {
            section: "knowledge".to_string(),
            content_markdown:
                "A Qdrant-backed knowledge base is available. Call `rag_search` before \
                 answering questions that depend on stored documents, and ground the \
                 answer in the returned passages rather than prior knowledge."
                    .to_string(),
            priority: 100,
        }]
    }
}

// ===== design::knowledge =====
impl knowledge::Guest for Component {
    fn list_entries(_category_filter: Option<String>) -> Vec<knowledge::EntrySummary> {
        // The knowledge interface is the designer's static, packaged
        // knowledge base. This extension's content lives in Qdrant at runtime
        // and is reached through `rag_search`, so there is nothing to list.
        Vec::new()
    }

    fn get_entry(id: String) -> Result<knowledge::Entry, types::ExtensionError> {
        Err(types::ExtensionError::NotFound(format!(
            "no packaged knowledge entry: {id}"
        )))
    }

    fn suggest_entries(_query: String, _limit: u32) -> Vec<knowledge::EntrySummary> {
        Vec::new()
    }
}

bindings::export!(Component with_types_in bindings);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_five_tools_reach_the_wit_layer_with_their_metadata_intact() {
        let listed = <Component as tools::Guest>::list_tools();
        assert_eq!(listed.len(), 5);
        for tool in &listed {
            assert!(
                tool.agentic_worker_metadata.is_some(),
                "{} lost its metadata crossing the WIT boundary",
                tool.name
            );
            let caps = tool.capabilities.clone().unwrap_or_default();
            assert!(caps.contains(&"flow".to_string()));
            assert!(caps.contains(&"agentic_worker".to_string()));
            assert!(tool.output_schema_json.is_some());
        }
    }

    /// The unknown-name guard must return before any secret is read or request
    /// sent — which is both correct and the only reason this is testable on the
    /// host at all.
    #[test]
    fn an_unknown_tool_is_rejected_before_any_host_call() {
        let err = <Component as tools::Guest>::invoke_tool("nope".to_string(), "{}".to_string())
            .unwrap_err();
        assert!(matches!(err, types::ExtensionError::InvalidInput(_)));
    }

    /// A known tool with unusable arguments must also fail during parsing,
    /// before the config lookup or any host call.
    #[test]
    fn malformed_arguments_are_rejected_before_any_host_call() {
        let err = <Component as tools::Guest>::invoke_tool(
            tool_meta::SEARCH_TOOL.to_string(),
            "{ not json".to_string(),
        )
        .unwrap_err();
        assert!(matches!(err, types::ExtensionError::InvalidInput(_)));
    }

    #[test]
    fn every_rag_error_maps_onto_a_distinct_extension_error() {
        use crate::error::RagError;
        assert!(matches!(
            map_error(RagError::InvalidInput("x".into())),
            types::ExtensionError::InvalidInput(_)
        ));
        assert!(matches!(
            map_error(RagError::PermissionDenied("x".into())),
            types::ExtensionError::PermissionDenied(_)
        ));
        assert!(matches!(
            map_error(RagError::NotFound("x".into())),
            types::ExtensionError::NotFound(_)
        ));
        assert!(matches!(
            map_error(RagError::SchemaInvalid("x".into())),
            types::ExtensionError::SchemaInvalid(_)
        ));
        assert!(matches!(
            map_error(RagError::Internal("x".into())),
            types::ExtensionError::Internal(_)
        ));
    }
}
