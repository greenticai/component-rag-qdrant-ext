//! The host boundary. This module is pure — it declares the shapes and the
//! trait; the only WIT-backed implementation lives in `lib.rs`.
//!
//! Everything that needs a host call takes `&impl HostCalls`. That is not
//! ceremony: calling a WIT import from a host `cargo test` aborts the process
//! (SIGABRT, non-unwinding, uncatchable), so injected host calls are the only
//! way this logic is testable at all.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

pub trait HostCalls {
    /// # Errors
    /// The host's transport error message. A non-2xx status is a successful
    /// `Ok` carrying that status — status handling belongs to the parsers.
    fn fetch(&self, req: &HttpRequest) -> Result<HttpResponse, String>;

    /// # Errors
    /// The host's message when the URI is undeclared or unresolvable.
    fn secret(&self, uri: &str) -> Result<String, String>;
}
