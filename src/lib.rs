// Design extension guest for greentic.rag-qdrant.
//
// This scaffold ships one working tool — `echo` — so a fresh build does
// something you can see: install it, and the designer lists `echo` and can
// invoke it. Replace it with your own tool rather than starting from a blank
// file.
//
// Every export below is required by the extension-design contract. The ones
// you do not need can stay as they are: returning an empty list is a valid
// answer meaning "this extension contributes nothing here".
//
// Two things to keep in step as you edit:
//   * `contributions` in describe.json is what the designer *lists*. A tool
//     implemented here but missing there is never offered to users.
//   * `gtdx lint --dir .` checks that pairing, plus tool naming and export
//     form. Run it before you publish.

#[allow(warnings)]
mod bindings;

pub mod chunk;
pub mod error;
pub mod config;
pub mod input;

use bindings::exports::greentic::extension_base::{lifecycle, manifest};
use bindings::exports::greentic::extension_design::{knowledge, prompting, tools, validation};
use bindings::greentic::extension_base::types;

struct Component;

// ---- extension-base/manifest ----
// Identity and capability wiring. The designer reads this to know who the
// component claims to be; it must agree with `metadata` in describe.json.
impl manifest::Guest for Component {
    fn get_identity() -> types::ExtensionIdentity {
        types::ExtensionIdentity {
            id: "greentic.rag-qdrant".to_string(),
            version: "0.1.0".to_string(),
            kind: types::Kind::Design,
        }
    }

    /// Capabilities other extensions may depend on, as `namespace:name`.
    /// Anything listed here must also appear in `capabilities.offered` in
    /// describe.json.
    fn get_offered() -> Vec<types::CapabilityRef> {
        Vec::new()
    }

    /// Capabilities this extension needs from others. The designer refuses to
    /// enable an extension whose required capabilities are unmet, so add an
    /// entry only when the extension genuinely cannot work without it.
    fn get_required() -> Vec<types::CapabilityRef> {
        Vec::new()
    }
}

// ---- extension-base/lifecycle ----
// Called once when the designer loads the component, and once at unload.
impl lifecycle::Guest for Component {
    /// `config_json` is the extension's settings object, serialised. Returning
    /// `Err` here stops the extension loading at all — reserve it for
    /// configuration that is genuinely unusable, not for missing optional
    /// values.
    fn init(_config_json: String) -> Result<(), types::ExtensionError> {
        Ok(())
    }

    fn shutdown() {
        // Release anything `init` acquired. A WASM component is torn down
        // wholesale, so this is only needed for work with side effects
        // outside the component (flushing a buffer, closing a session).
    }
}

// ---- extension-design/tools ----
// Tools are the verbs this extension adds. The designer lists them, an agentic
// worker may call them, and each call arrives here as `invoke_tool`.
impl tools::Guest for Component {
    /// Describe every tool this extension implements.
    ///
    /// `input_schema_json` is a JSON Schema object. The designer renders it as
    /// the tool's form and validates arguments against it before calling, so a
    /// precise schema here means fewer bad inputs to handle below.
    fn list_tools() -> Vec<tools::ToolDefinition> {
        vec![tools::ToolDefinition {
            // snake_case, and unique within this extension. `gtdx lint`
            // rejects camelCase and near-duplicate names.
            name: "echo".to_string(),
            // Shown in the designer's tool list. Write it for the person
            // choosing between tools, not for the implementer.
            description: "Return the message that was passed in.".to_string(),
            input_schema_json: r#"{
                "type": "object",
                "properties": {
                    "message": { "type": "string", "title": "Message" }
                },
                "required": ["message"]
            }"#
            .to_string(),
            // Optional. Declare it when callers benefit from knowing the
            // result shape ahead of time; `none` means "unspecified".
            output_schema_json: Some(
                r#"{"type":"object","properties":{"echoed":{"type":"string"}}}"#.to_string(),
            ),
            // Where the tool may be used. `flow` = as a flow node,
            // `agentic_worker` = callable by an agent. `None` means the
            // consumer defaults to `["flow"]`.
            capabilities: Some(vec!["flow".to_string(), "agentic_worker".to_string()]),
            agentic_worker_metadata: None,
        }]
    }

    /// Run one tool. `args_json` is the caller's arguments, already validated
    /// against the schema above; the return value is the JSON-encoded result.
    ///
    /// Match on `name` and fall through to an error — an unknown name is a
    /// caller mistake worth reporting, not something to answer with an empty
    /// success.
    fn invoke_tool(name: String, args_json: String) -> Result<String, types::ExtensionError> {
        match name.as_str() {
            // Deliberately dependency-free: the scaffold ships no JSON
            // library, so this hands the arguments straight back. Real tools
            // will want to parse `args_json` — add `serde_json` to Cargo.toml
            // and deserialise into a struct.
            // Built by concatenation rather than `format!`: a Rust format
            // string would have to escape its braces by doubling them, and
            // doubled braces are the scaffold renderer's own placeholder
            // syntax — the file would never render.
            "echo" => Ok(["{\"echoed\":", &args_json, "}"].concat()),
            _ => Err(types::ExtensionError::InvalidInput(format!(
                "unknown tool: {name}"
            ))),
        }
    }
}

// ---- extension-design/validation ----
// Domain checks the designer runs against user content, so mistakes surface
// while editing rather than at deploy time.
impl validation::Guest for Component {
    /// `content_type` says what is being validated; `content_json` is the
    /// content itself. Return `valid: false` with diagnostics to block, or
    /// `valid: true` with diagnostics to warn without blocking.
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

// ---- extension-design/prompting ----
// Text merged into the designer's LLM system prompt. Use it to teach the model
// vocabulary specific to this extension.
impl prompting::Guest for Component {
    /// `section` groups related fragments; `priority` orders them, lower
    /// first. Keep fragments short — every one costs context on every request.
    fn system_prompt_fragments() -> Vec<prompting::PromptFragment> {
        Vec::new()
    }
}

// ---- extension-design/knowledge ----
// Reference material the designer can search and show. Optional: an extension
// that contributes no documentation leaves all three returning empty.
impl knowledge::Guest for Component {
    fn list_entries(_category_filter: Option<String>) -> Vec<knowledge::EntrySummary> {
        Vec::new()
    }

    fn get_entry(id: String) -> Result<knowledge::Entry, types::ExtensionError> {
        Err(types::ExtensionError::InvalidInput(format!(
            "unknown entry: {id}"
        )))
    }

    /// Ranked lookup for `query`, capped at `limit`. Called as the user types,
    /// so keep it cheap.
    fn suggest_entries(_query: String, _limit: u32) -> Vec<knowledge::EntrySummary> {
        Vec::new()
    }
}

bindings::export!(Component with_types_in bindings);

// Unit tests run on the host — no WASM, no designer, milliseconds. This is
// where most of an extension's testing belongs: the guest exports are plain
// Rust functions, so call them directly.
//
// `cargo test` needs `src/bindings.rs`, which is generated. Build once first
// (`gtdx dev --once` or `cargo component build`), then test freely.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tool_is_listed() {
        let tools = <Component as tools::Guest>::list_tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");
    }

    #[test]
    fn echo_returns_its_arguments() {
        let out = <Component as tools::Guest>::invoke_tool(
            "echo".to_string(),
            r#"{"message":"hi"}"#.to_string(),
        )
        .expect("echo is implemented");
        assert!(out.contains("echoed"), "got: {out}");
    }

    /// An unknown name is a caller mistake. Asserting the error keeps the
    /// dispatch honest — a match arm that silently returned Ok would pass a
    /// happy-path-only test.
    #[test]
    fn an_unknown_tool_is_an_error() {
        assert!(
            <Component as tools::Guest>::invoke_tool("nope".to_string(), "{}".to_string())
                .is_err()
        );
    }
}
