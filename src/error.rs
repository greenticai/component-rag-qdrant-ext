//! The extension's own error type. Pure: `lib.rs` maps this onto the WIT
//! `extension-error` at the boundary so no other module needs bindings.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RagError {
    /// Malformed arguments, contradictory arguments, or missing configuration.
    InvalidInput(String),
    /// Host refused a secret or an out-of-allowlist URL; upstream 401/403.
    PermissionDenied(String),
    /// Collection or point absent; upstream 404.
    NotFound(String),
    /// Embedding dimensions disagree with the collection.
    SchemaInvalid(String),
    /// Transport failure, 5xx, or an unparseable response body.
    Internal(String),
}

impl core::fmt::Display for RagError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidInput(m) => write!(f, "invalid input: {m}"),
            Self::PermissionDenied(m) => write!(f, "permission denied: {m}"),
            Self::NotFound(m) => write!(f, "not found: {m}"),
            Self::SchemaInvalid(m) => write!(f, "schema invalid: {m}"),
            Self::Internal(m) => write!(f, "internal: {m}"),
        }
    }
}
