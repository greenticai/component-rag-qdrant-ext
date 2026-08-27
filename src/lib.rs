//! Greentic RAG/Qdrant design extension — WIT export layer.
//!
//! Exports six tools (`rag_search`, `rag_upsert`, `rag_ingest`, `rag_delete`,
//! `rag_collection_ensure`, `rag_list`) via `greentic:extension-design/tools`. Every piece of
//! logic lives in the pure modules below; this file is the only one that may
//! touch `bindings`, because a host `cargo test` that reaches a WIT import
//! aborts the process outright.
#![allow(clippy::used_underscore_items)]

#[allow(warnings)]
#[rustfmt::skip]
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

/// Parse arguments, resolve this call's configuration, then run the operation.
///
/// Argument parsing happens first so a malformed call fails the same way
/// whether or not the extension is configured — and because the configuration
/// itself is *inside* the arguments: the host stamps this tenant's effective
/// config onto every call as `_tenant_overlay`, and it can only be read once
/// the arguments have been decoded. `config::installed()` is the optional
/// `lifecycle::init` baseline underneath it, normally `None`; see
/// [`config::resolve`] for the precedence.
///
/// The resolved `Config` is owned and per-call, not a borrow of a process-wide
/// static: one instance serves many tenants and no two consecutive calls are
/// guaranteed to belong to the same one.
fn dispatch(name: &str, args_json: &str) -> Result<String, RagError> {
    let host = WitHost;
    let base = config::installed();
    let value = match name {
        tool_meta::SEARCH_TOOL => {
            let input = input::parse_search(args_json)?;
            let cfg = config::resolve(base, input.tenant_overlay.as_ref())?;
            ops::search(&host, &cfg, &input)?
        }
        tool_meta::UPSERT_TOOL => {
            let input = input::parse_upsert(args_json)?;
            let cfg = config::resolve(base, input.tenant_overlay.as_ref())?;
            ops::upsert(&host, &cfg, &input)?
        }
        tool_meta::INGEST_TOOL => {
            let input = input::parse_ingest(args_json)?;
            let cfg = config::resolve(base, input.tenant_overlay.as_ref())?;
            ops::ingest(&host, &cfg, &input)?
        }
        tool_meta::DELETE_TOOL => {
            let input = input::parse_delete(args_json)?;
            let cfg = config::resolve(base, input.tenant_overlay.as_ref())?;
            ops::delete(&host, &cfg, &input)?
        }
        tool_meta::ENSURE_TOOL => {
            let input = input::parse_ensure(args_json)?;
            let cfg = config::resolve(base, input.tenant_overlay.as_ref())?;
            ops::ensure_collection(&host, &cfg, &input)?
        }
        tool_meta::LIST_TOOL => {
            let input = input::parse_list(args_json)?;
            let cfg = config::resolve(base, input.tenant_overlay.as_ref())?;
            ops::list(&host, &cfg, &input)?
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
mod view_asset_tests {
    //! `contributions.views[]` declares two entries — one per host surface —
    //! and `gtdx lint` resolves each entry under `assets/views/<view id>/`,
    //! so each id needs its own real directory. The packer copies only real
    //! files (a symlinked view directory lints clean and then ships *nothing*,
    //! which is a lint-clean broken install), so the two surfaces are served
    //! by two byte-identical copies of the same page.
    //!
    //! Two copies that must stay identical is a drift hazard, so it is checked
    //! rather than trusted. `include_str!` resolves at compile time, so this
    //! needs no filesystem access when the test runs.

    macro_rules! assert_view_copies_match {
        ($($file:literal),+ $(,)?) => {
            $(
                assert_eq!(
                    include_str!(concat!("../assets/views/knowledge/", $file)),
                    include_str!(concat!("../assets/views/knowledge-admin/", $file)),
                    concat!(
                        $file,
                        " differs between assets/views/knowledge/ and \
                         assets/views/knowledge-admin/. Both surfaces must serve the same \
                         page; copy the changed file across."
                    )
                );
            )+
        };
    }

    #[test]
    fn the_designer_and_admin_copies_of_the_view_are_identical() {
        assert_view_copies_match!("index.html", "app.js", "bridge.js", "pdf.js", "style.css");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_six_tools_reach_the_wit_layer_with_their_metadata_intact() {
        let listed = <Component as tools::Guest>::list_tools();
        assert_eq!(listed.len(), 6);
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

    /// The bug this file's dispatch change exists to fix, pinned at the only
    /// boundary the host actually calls.
    ///
    /// `lifecycle::init` has not run in this test binary and never will (see
    /// `config`'s `no_baseline_is_installed_until_init_runs`), which is also
    /// the state of every real deployment — the host runtime has no
    /// init/configure entry point. Before this change every tool call in that
    /// state died on "extension is not configured — lifecycle::init has not
    /// run", naming an entry point the operator cannot invoke. It must now
    /// name the console and the fields instead.
    ///
    /// Safe to run on the host: resolution fails before `ops` reaches a WIT
    /// import, and a reached import would abort the whole test binary.
    #[test]
    fn an_unconfigured_call_is_refused_with_an_operator_actionable_message() {
        let err = <Component as tools::Guest>::invoke_tool(
            tool_meta::SEARCH_TOOL.to_string(),
            r#"{"query":"anything"}"#.to_string(),
        )
        .unwrap_err();
        let types::ExtensionError::InvalidInput(msg) = err else {
            panic!("expected InvalidInput, got {err:?}");
        };
        assert!(msg.contains("admin console"), "message was: {msg}");
        assert!(msg.contains("qdrant_url"), "message was: {msg}");
        assert!(msg.contains("collection"), "message was: {msg}");
        assert!(!msg.contains("lifecycle::init"), "message was: {msg}");
    }

    /// The other half: an overlay-only call gets *past* configuration with no
    /// `lifecycle::init` anywhere, and the tenant-isolation refusal still
    /// fires on the far side of it.
    ///
    /// Reaching that refusal is the proof — it lives in `ops::collection_of`,
    /// after `config::resolve` has accepted the overlay as this call's whole
    /// configuration. A call that fell at the configuration hurdle instead
    /// would report the unconfigured error above, not this one.
    #[test]
    fn an_overlay_configures_the_call_and_the_isolation_refusal_still_fires() {
        let err = <Component as tools::Guest>::invoke_tool(
            tool_meta::SEARCH_TOOL.to_string(),
            r#"{"vector":[0.1,0.2,0.3],"collection":"tenant-b",
                "_tenant_overlay":{"qdrant_url":"https://t.qdrant.io:6333",
                                   "collection":"tenant-a"}}"#
                .to_string(),
        )
        .unwrap_err();
        let types::ExtensionError::InvalidInput(msg) = err else {
            panic!("expected InvalidInput, got {err:?}");
        };
        assert!(
            msg.contains("tenant-b") && msg.contains("tenant-a"),
            "expected the caller-collection refusal, got: {msg}"
        );
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
