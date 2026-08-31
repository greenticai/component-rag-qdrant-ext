# How to make a RAG extension

This is a build tutorial, not a reference. It walks through building a Greentic
Designer RAG extension from nothing, in the order the pieces actually depend
on each other, using this repository (`greentic.rag-qdrant`, a Qdrant-backed
extension) as the worked example. Qdrant is incidental — the decisions below
apply to Pinecone, Weaviate, pgvector, or anything else with an HTTP API and a
notion of "insert a vector with an id."

Two documents already cover this repo from other angles, and this one leans
on both rather than repeating them:

- [`README.md`](../README.md) — how to *call* this extension: quick start,
  the six-tool reference, a worked example, and the tenant-isolation model.
- [`docs/ARCHITECTURE.md`](ARCHITECTURE.md) — what every tracked file is for,
  file by file, plus its own condensed [build
  walkthrough](ARCHITECTURE.md#building-one-yourself). If this tutorial is the
  narrated version, that walkthrough is the checklist version — useful once
  you've built one of these before.

What follows is the narrative: not just *what* to type, but *why* each piece
exists, illustrated with the actual bug each decision fixed here. Every
command below was run against this repository while writing this document;
every code excerpt is quoted, not reconstructed from memory.

## 0. The constraint that decides your architecture before you write a tool

Before scope, before schemas, know this one fact, because it decides how you
structure the whole crate:

A Greentic design extension compiles to `wasm32-wasip2`. Its host calls —
`http::fetch`, `secrets::get`, and friends — are WIT imports, satisfied at
runtime by the host that loads the component. Call one of those imports from
a plain host `cargo test` (no WASM runtime underneath) and the process
**aborts with `SIGABRT`** — non-unwinding, uncatchable, nothing to `Result`
or even `catch_unwind` on, and it takes the *entire test binary* down with
it, not just the one test.

So nothing outside one file may call a WIT binding directly. This repo's
answer is `src/host.rs`, a trait standing in for every host call:

```rust
pub trait HostCalls {
    /// # Errors
    /// The host's transport error message. A non-2xx status is a successful
    /// `Ok` carrying that status — status handling belongs to the parsers.
    fn fetch(&self, req: &HttpRequest) -> Result<HttpResponse, String>;

    /// # Errors
    /// The host's message when the URI is undeclared or unresolvable.
    fn secret(&self, uri: &str) -> Result<String, String>;
}
```

Every module that needs a host call — the embeddings client, the vector-store
client, orchestration — takes `&impl HostCalls` generically instead of
calling `bindings::` directly. `src/lib.rs` is the *only* module allowed to
import `crate::bindings`, and it provides the one real implementation:

```rust
pub struct WitHost;

impl HostCalls for WitHost {
    fn fetch(&self, req: &HttpRequest) -> Result<HttpResponse, String> {
        let wire = bindings::greentic::extension_host::http::Request { /* ... */ };
        let resp = bindings::greentic::extension_host::http::fetch(&wire)?;
        Ok(HttpResponse { status: resp.status, body: resp.body })
    }

    fn secret(&self, uri: &str) -> Result<String, String> {
        bindings::greentic::extension_host::secrets::get(uri)
    }
}
```

Tests substitute `MockHttpClient` / `MockSecretsBackend` from
`greentic-extension-sdk-testing` instead of `WitHost`. That's what makes 136
tests run in milliseconds without a WASM runtime or a live vector store — and
it's why the module list below is ordered the way it is: everything gets
built pure and host-testable first, and the WIT glue is the very last thing
you write, not the first.

If you skip this and put a `bindings::` call in a pure module, `cargo check`
will not warn you — it compiles fine. It SIGABRTs the instant a test reaches
it, with no useful backtrace. Get the boundary right before writing your
first tool.

## 1. Decide the tool surface before writing Rust

Design extensions declare tools in one file — here, `src/tool_meta.rs` — as
`&'static str` schema constants plus a catalog function. Decide the surface
up front because everything downstream (input parsing, orchestration,
tool metadata) is written against it:

- **`rag_search`** — semantic search, read-only.
- **`rag_upsert`** — store or replace one point by id, for callers who
  already have a vector or a short fact.
- **`rag_ingest`** — chunk, embed and store a whole document under a
  `doc_id`, replacing on re-ingest.
- **`rag_delete`** — remove points, by id or by `doc_id`.
- **`rag_collection_ensure`** — create the collection with a fixed vector
  width and distance metric.
- **`rag_list`** — enumerate stored documents, paginated.

Two things about this surface are worth deciding now rather than discovering
later, because both come back in step 8 and step 11.

**Either/or arguments need a formal rule, not just prose.** `rag_search` and
`rag_upsert` accept a `text`/`query` field *or* a pre-computed `vector`,
never both, never neither. `rag_delete` accepts `ids` *or* `doc_id`. A model
that only reads a schema's `required`/`properties` — and many do, ignoring
`oneOf` — will keep constructing invalid calls (both fields, or neither) and
retrying forever if the schema doesn't say so structurally. Decide the
either/or pairs now; you'll encode them twice, once in JSON Schema and once
in Rust (step 8).

**Every `agentic_worker` tool needs real metadata, or the runtime picks
worst-case for you.** A tool that declares the `agentic_worker` capability
but ships no `usage_hint`/`side_effects`/`cost`/`confirmation_required` isn't
left blank — the SDK fills it in. From
`greentic-extension-sdk-contract`'s `AgenticWorkerMetadata`:

```rust
/// Returns a new copy with conservative defaults applied to any field
/// the extension left as `None`. Per spec: when a tool declares
/// `agentic_worker` capability but ships no metadata, runtime treats
/// it as `External` side-effects + `confirmation_required: true` until
/// the extension declares otherwise.
pub fn with_conservative_defaults(mut self) -> Self {
    if self.side_effects.is_none() {
        self.side_effects = Some(SideEffects::External);
    }
    if self.confirmation_required.is_none() {
        self.confirmation_required = Some(true);
    }
    if self.cost.is_none() {
        self.cost = Some(Cost::Medium);
    }
    self
}
```

Leave `rag_search` — a read-only lookup — without metadata, and an agentic
worker will prompt the user for confirmation on *every single search call*.
Decide, per tool, whether it reads or writes, and whether a mistake is
recoverable, before you write `tool_meta.rs`; it is far easier to get right
once than to notice later that every read has been silently asking for
permission.

## 2. Scaffold

```
gtdx new <name> --kind design --id <reverse-dns-id>
```

(`gtdx new --help`, run against `gtdx 1.2.7` in this environment, confirms
this shape; nothing here is invented.) Run without a name on a terminal for
an interactive wizard instead.

This produces the skeleton every later step fills in: `describe.json` with
sample metadata and one placeholder tool, `wit/world.wit` plus a vendored,
locked `wit/deps/`, a starter `src/lib.rs`, `Cargo.toml`, `i18n/en.json`,
`build.sh`, `ci/local_check.sh`, `rust-toolchain.toml`, and the
`AGENTS.md`/`CLAUDE.md`/`.claude/` agent-config trio.

**Get this right immediately, because it's expensive to fix later:** the id
and its WIT-package form must stay in sync in four places for the life of
the project — `describe.json`'s `metadata.id` and `runtime.components` key,
`Cargo.toml`'s `package.metadata.component.package`, and `wit/world.wit`'s
`package` line. In this repo they are `greentic.rag-qdrant` and
`greentic:rag-qdrant` respectively. Renaming later means touching all four
together.

## 3. Trim the WIT world to what you actually import

`gtdx new` scaffolds a world that imports every host interface the SDK
offers. Trim it. This repo's `wit/world.wit`:

```wit
package greentic:rag-qdrant;

world extension {
  import greentic:extension-base/types@0.2.0;
  import greentic:extension-host/logging@0.1.0;
  import greentic:extension-host/i18n@0.1.0;
  import greentic:extension-host/secrets@0.1.0;
  import greentic:extension-host/broker@0.1.0;
  import greentic:extension-host/http@0.1.0;

  export greentic:extension-base/manifest@0.2.0;
  export greentic:extension-base/lifecycle@0.2.0;
  export greentic:extension-design/tools@0.3.0;
  export greentic:extension-design/validation@0.3.0;
  export greentic:extension-design/prompting@0.3.0;
  export greentic:extension-design/knowledge@0.3.0;
}
```

Notice what's *not* imported: `greentic:extension-host/llm`. That's not an
oversight — it's the first RAG-specific decision, and it's worth stating
plainly before writing any Rust: **the host gives you no embeddings.**

## 4. The host gives you no embeddings

Every other WIT interface in this SDK looks like it might help until you
read it closely. `extension-host/llm` is the one that seems most promising
for a RAG extension and turns out not to help at all:

```wit
interface llm {
  record llm-message { role: string, content: string }
  variant response-format { text, json, json-schema(string) }
  record llm-request {
    role-hint: option<string>,
    system-prompt: string,
    messages: list<llm-message>,
    response-format: option<response-format>,
  }
  record llm-response { content: string, total-tokens: option<u32> }
  complete: func(request: llm-request) -> result<llm-response, string>;
}
```

One function: `complete`. Chat completion, not embeddings. There is no
`embed` anywhere in the host interfaces this SDK exposes. If your tool needs
a vector, you are the one who has to produce it.

The consequence shows up in three places, and it's worth tracing all three
now rather than hitting them one at a time:

1. **An extra secret.** This extension resolves its own embeddings API key —
   `secret://rag-qdrant/embedding_api_key` — completely separately from the
   Qdrant key. Two credentials to configure, not one.
2. **An extra network allowlist entry.** `runtime.permissions.network` has to
   list the embeddings host in addition to the vector store's host (see step
   13) — an unlisted host means `http::fetch` is refused before the request
   ever leaves the sandbox.
3. **A config surface for base URL, model, and dimensions**, because "the
   embeddings API" isn't one fixed service — see `Config::embedding` in step
   7.

The client itself is a pure module, `src/embed.rs`, built the same way as
every other module: a request builder plus a response parser, no
`bindings::` call in sight.

```rust
/// Build the `POST {base_url}/embeddings` request for a batch of inputs.
pub fn embed_request(cfg: &EmbeddingConfig, inputs: &[String], api_key: &str) -> HttpRequest {
    let body = serde_json::json!({ "model": cfg.model, "input": inputs });
    HttpRequest {
        method: "POST".to_string(),
        url: format!("{}/embeddings", cfg.base_url),
        headers: vec![
            ("authorization".to_string(), format!("Bearer {api_key}")),
            ("content-type".to_string(), "application/json".to_string()),
        ],
        body: Some(serde_json::to_vec(&body).unwrap_or_default()),
    }
}
```

The response parser is worth reading past the happy path, because the bug it
guards against is subtle. An OpenAI-shaped embeddings response returns
vectors tagged with an `index`, and the naive approach — sort by index, then
zip with the input list — has a hole: two items both claiming the same index
sort adjacent and pass a plain length check, silently pairing one chunk's
text with *another chunk's* vector.

```rust
let mut items = parsed.data;
items.sort_by_key(|item| item.index);

// Sorting alone does not prove the indices are `0..n`. Two items both
// reporting the same index would sort adjacent and pass the length check
// in `embed_all`, silently pairing one chunk's text with another's
// vector. Require the indices to be exactly `0..n` — contiguous, no
// duplicates, no gaps.
if items.iter().enumerate().any(|(i, item)| item.index != i) {
    return Err(RagError::Internal(format!(
        "embeddings response indices are not contiguous from 0: {:?}",
        items.iter().map(|item| item.index).collect::<Vec<_>>()
    )));
}
```

`parse_embed_response` also maps 401/403 to `PermissionDenied` and a
dimension mismatch against the configured width to `SchemaInvalid` — the
same error taxonomy every pure module uses (step 6).

## 5. The host gives you no filesystem

The second RAG-specific gap, and it follows the same shape as the first:
there is no `filesystem` import in this world either, and there won't be one
in yours unless your extension declares a reason to need it. That means
`rag_ingest` cannot mean "read a file" — it takes `text: string`, full stop.

Whatever converts a PDF or a DOCX into text has to happen *before* the tool
call, outside the WASM sandbox. This repo's answer is a contributed browser
view (`assets/views/knowledge/`) that extracts text in the page — plain text
and Markdown decode directly, PDF goes through a dependency-free extractor
(`pdf.js`) that rejects rather than guesses on a bad decode. DOCX is not
supported anywhere in this repository. The full format table and the reasons
behind "reject, don't guess" are in
[README.md § Reading files in the browser](../README.md#reading-files-in-the-browser)
— worth reading before you build a view, not reproduced here.

The takeaway that generalizes past this repo: if your extension's ingest
path is meant to take arbitrary documents rather than plain text, decide
now whether that conversion happens in a contributed view (browser-side,
after the sandbox), by asking the caller to convert before calling your
tool, or not at all for that format. There is no fourth option inside the
extension itself.

## 6. The error taxonomy

Small, but every pure module needs it before it can return anything, so
write it now. `src/error.rs`:

```rust
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
```

Only `src/lib.rs` translates this onto the WIT `extension-error` variant
(step 12), so no other module needs `bindings::` just to report a failure.
Because the match in that translation has to stay exhaustive, adding a
variant here without handling it there is a compile error — the real risk
isn't a missing arm, it's picking the wrong *existing* variant for a new
failure and quietly changing how a caller is expected to react to it.

## 7. Operator configuration

`src/config.rs` holds the wire shape (every field optional — it is what the
host stamps onto each call as `_tenant_overlay`, and also what the optional
`lifecycle::init` receives) and the resolved shape it merges into. Do not build
this around `lifecycle::init` alone: no host calls it. The resolved shape is:

```rust
/// The outcome of resolution: every field present, every field validated.
/// Not `Deserialize` — nothing ever arrives in this shape.
pub struct Config {
    pub qdrant_url: String,
    pub collection: String,
    pub embedding: EmbeddingConfig,
    pub chunk: ChunkConfig,
    pub require_tenant_overlay: bool,
}

pub struct EmbeddingConfig {
    pub base_url: String,
    pub model: String,
    pub dimensions: u32,
}

/// The wire shape, on both channels. `Deserialize`, all-optional, unknown keys
/// ignored — a tenant override sets only the fields it changes, and a host that
/// learns to send more must not break a guest that has not learned to read it.
#[derive(Deserialize)]
pub struct ConfigOverlay {
    pub qdrant_url: Option<String>,
    pub collection: Option<String>,
    pub embedding: Option<EmbeddingOverlay>,
    pub chunk: Option<ChunkOverlay>,
    pub require_tenant_overlay: Option<bool>,
}

pub fn resolve(
    base: Option<&Config>,             // the optional lifecycle::init baseline
    overlay: Option<&ConfigOverlay>,   // this call's _tenant_overlay
) -> Result<Config, RagError>;
```

`EmbeddingConfig` exists purely because of step 4 — base URL, model, and
vector width are all things an operator has to be able to point at a
different provider or a different model without a code change.

Two validation details worth carrying into your own config module:

- **Strip trailing slashes from every URL.** Every request builder joins the
  configured base with a leading-slash path; an unstripped trailing slash
  produces `//embeddings` or `//collections/...`, which some proxies accept
  and others 404 on.
- **Reject `overlap_chars >= max_chars` at config time**, not just at chunk
  time — see step 8 for why the chunker needs a second, independent guard
  against the same condition.

The re-init policy is its own small decision: a `OnceLock` can't be
replaced, so a second `init` call with an *identical* config succeeds as a
harmless reload, and a second call with a *different* config is rejected
outright rather than silently ignored — an operator who believes a config
change took effect must not be lied to.

## 8. Chunking

One pure function, `src/chunk.rs`, splitting text into overlapping character
windows:

```rust
pub fn chunk_text(text: &str, max_chars: usize, overlap_chars: usize) -> Vec<String> {
    if text.trim().is_empty() || max_chars == 0 {
        return Vec::new();
    }
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return vec![text.to_string()];
    }

    // `max(1)` is load-bearing: an overlap >= max_chars would give a step of 0
    // and spin forever. Clamping degrades the overlap rather than hanging.
    let step = max_chars.saturating_sub(overlap_chars).max(1);
    // ...
}
```

Two things here generalize past this repo. Operate on `Vec<char>`, not
bytes — a byte window can split a multi-byte UTF-8 character in half.
And clamp the step to at least 1 independently of whatever your config
validation already checks: `config.rs` only rejects `overlap_chars >=
max_chars`, not the exact boundary condition that makes the step zero, so
the clamp here is a second, cheaper line of defense against an infinite
loop rather than trusting the caller upstream to have gotten it right.

## 9. Input parsing — enforce the either/or rule twice

Recall from step 1: a JSON Schema `oneOf` alone doesn't stop a model that
only reads `required`/`properties`. `src/input.rs` re-enforces every
either/or rule in Rust, one parser per tool:

```rust
/// Exactly one of a text field and a vector field must be present. Both means
/// the caller has two conflicting intents and we would have to silently pick
/// one; neither means there is nothing to search or store.
fn exactly_one(tool: &str, has_text: bool, has_vector: bool) -> Result<(), RagError> {
    match (has_text, has_vector) {
        (true, false) | (false, true) => Ok(()),
        (true, true) => Err(RagError::InvalidInput(format!(
            "{tool}: pass either a text field or `vector`, not both"
        ))),
        (false, false) => Err(RagError::InvalidInput(format!(
            "{tool}: pass either a text field or `vector`"
        ))),
    }
}
```

`rag_search`, `rag_upsert`, and `rag_delete` each call this (or its
`ids`/`doc_id` equivalent) right after decoding. A model that skips `oneOf`
and sends both fields — or neither — gets a clear `InvalidInput` back
instead of the extension silently guessing which one it meant.

The other input-side decision is specific to any vector store with
constraints on what a point id can be. Qdrant accepts only an unsigned
integer or a UUID:

```rust
/// Qdrant accepts only an unsigned integer or a UUID as a point id; anything
/// else is a 400 from Qdrant, not a value we should spend an embeddings call
/// and an ensure PUT on first.
fn validate_point_id(id: &str) -> Result<(), RagError> {
    if id.parse::<u64>().is_ok() || Uuid::parse_str(id).is_ok() {
        return Ok(());
    }
    Err(RagError::InvalidInput(format!(
        "rag_upsert: id {id:?} must be an unsigned integer or a UUID"
    )))
}
```

This runs in `parse_upsert`, before any host call. The comment states the
reason precisely: without it, a caller's malformed id would only surface as
an opaque 400 from Qdrant *after* an embeddings call had already been spent.
Whatever store you build against, find its id constraint and check it here,
before the network call that would otherwise pay for a request you're about
to throw away.

## 10. The vector-store client

`src/qdrant.rs` is the one module in this repository that is actually
Qdrant-specific — see the closing section for exactly what that means for
porting to another store. It's pure request builders and response parsers,
same shape as `embed.rs`: `ensure_collection_request`, `upsert_request`,
`query_request`, `scroll_request`, `delete_request`, and parsers for each.

The one piece of logic worth its own section is `chunk_point_id`, because it
is the fix for a real duplication bug:

```rust
const CHUNK_NAMESPACE: Uuid = Uuid::from_bytes([
    0x6b, 0xa7, 0xb8, 0x12, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30, 0xc8,
]);

pub fn chunk_point_id(doc_id: &str, chunk_index: usize) -> String {
    Uuid::new_v5(&CHUNK_NAMESPACE, format!("{doc_id}:{chunk_index}").as_bytes()).to_string()
}
```

`CHUNK_NAMESPACE` is the RFC 4122 OID namespace UUID, used here only because
nothing else in the crate reuses it — there is nothing semantically special
about it, and any fixed namespace constant would do as long as it never
changes.

The reason this exists at all: point ids for the same `(doc_id,
chunk_index)` pair have to be **deterministic and stable across re-ingests**.
Compute a random id per chunk instead, and re-ingesting the same document
twice produces two full sets of chunks in the collection instead of one —
duplicates that both match every future search, forever. UUIDv5 makes
`chunk_point_id("greentic-overview", 0)` return the exact same string every
time it's called, so an upsert with that id *overwrites* the previous chunk
in place. This is also why changing `CHUNK_NAMESPACE` later is a one-way
door: it orphans every point already written by every install running an
older version, silently splitting old and new chunk ids for the same
document.

## 11. Orchestration — the only module that sequences host calls

`src/ops.rs` is where chunking, embedding, and the vector-store client come
together, and it's the only module in the crate allowed to call more than
one host operation for a single tool. That restriction is deliberate:
ordering bugs can only exist where more than one call happens in sequence,
so confining multi-call sequences to one file is what makes those bugs easy
to find and to test.

`rag_ingest` is the tool where getting the order wrong is easiest, and where
this repo got it wrong once before fixing it. The final order is:

```rust
let chunks = chunk_text(&input.text, cfg.chunk.max_chars, cfg.chunk.overlap_chars);
// ...
let vectors = embed_all(host, cfg, &chunks)?;
let key = secret(host, QDRANT_KEY_REF)?;

ensure(host, cfg, collection, cfg.embedding.dimensions, "Cosine", &key)?;

// Delete first. See the doc comment above — the order is the point.
let del = delete_request(&cfg.qdrant_url, collection, &DeleteSelector::DocId(input.doc_id.clone()), &key);
let del_resp = send(host, &del)?;
parse_ack(del_resp.status, &del_resp.body)?;

let points: Vec<Point> = /* build one Point per chunk, id = chunk_point_id(doc_id, index) */;
let req = upsert_request(&cfg.qdrant_url, collection, &points, &key);
```

chunk → embed → ensure collection → **delete the document's existing chunks
→ upsert the new ones**. Two ordering rules are packed into this, and both
are pinned by a test that asserts *positions in the mock's recorded call
list*, not just that both calls eventually happened
(`ingest_deletes_the_document_before_upserting_its_chunks`):

- **Delete before upsert, not after.** If a document shrinks from 5 chunks
  to 3 on re-ingest and the delete ran last (or was skipped), the old chunks
  3 and 4 survive with stale content and keep matching every future search
  — orphans with no way for a caller to ever notice they're stale. Deleting
  first, by `doc_id`, removes every old chunk regardless of how many there
  used to be, before any new one is written.
- **Embed before delete, not after.** If the embeddings call fails and the
  delete had already run, a failed re-ingest leaves the knowledge base with
  *nothing* under that `doc_id` — worse than before the call. Embedding
  first means a failure stops execution before the delete runs, leaving the
  previous, working version of the document intact.

Both rules generalize past Qdrant: any store where "ingest" means
"chunk + write, replacing what was there before" needs delete-before-upsert
to avoid orphans and embed-before-delete to avoid destroying a working
document on a transient failure. Neither is something the store enforces
for you — it's the orchestration module's job.

`ops.rs` also owns tenant-collection resolution (`collection_of`), which
decides *which* collection a call reads and writes before any of the above
runs — a refusal must not first cost the operator a billable embeddings
call. That mechanism is specific to multi-tenant hosting and is covered in
full in
[README.md § Collections and tenancy](../README.md#collections-and-tenancy);
skip it for a single-tenant extension and come back to it if you need one.

## 12. Tool metadata

`src/tool_meta.rs` is the file you actually designed back in step 1 — now
write it. Each schema encodes its either/or rule formally, not just in a
field description:

```rust
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
```

Two branches, each requiring one field and explicitly excluding the other —
that's what stops a model from constructing a call with both or neither and
retrying in a loop, on top of the Rust-side enforcement from step 9.

Agentic-worker metadata follows the calculus from step 1 directly: reads
don't prompt, writes do.

```rust
const SEARCH_META: &str = r#"{
  "usage_hint": "Retrieve passages from the knowledge base by meaning. ...",
  "side_effects": "read",
  "cost": "low",
  "confirmation_required": false
}"#;

const INGEST_META: &str = r#"{
  "usage_hint": "Upload a whole document: it is chunked, embedded, and stored under doc_id. ...",
  "side_effects": "write",
  "cost": "medium",
  "confirmation_required": true
}"#;
```

`all_tools()` pairs each tool with `capabilities: both()` — `["flow",
"agentic_worker"]` — so every tool is reachable from a designer flow node
*and* from the agentic worker. A test in this repo
(`reads_do_not_ask_for_confirmation_and_writes_do`) asserts exactly the
split above across all six tools: `rag_search`, `rag_upsert`, and
`rag_list` never prompt; `rag_ingest`, `rag_delete`, and
`rag_collection_ensure` always do. Encode that as a test, not just a
convention — it's the kind of thing that's easy to get right once and
regress silently on tool number seven.

## 13. The WIT glue

`src/lib.rs` is the last module you write, and it should be thin — glue, not
logic. Two things happen here: implementing the WIT-exported `Guest` traits,
and dispatching each tool name to the matching `ops::*` function.

```rust
fn dispatch(name: &str, args_json: &str) -> Result<String, RagError> {
    let host = WitHost;
    let base = config::installed(); // the optional lifecycle::init baseline; normally None
    let value = match name {
        tool_meta::SEARCH_TOOL => {
            let input = input::parse_search(args_json)?;
            let cfg = config::resolve(base, input.tenant_overlay.as_ref())?;
            ops::search(&host, &cfg, &input)?
        }
        tool_meta::INGEST_TOOL => {
            let input = input::parse_ingest(args_json)?;
            let cfg = config::resolve(base, input.tenant_overlay.as_ref())?;
            ops::ingest(&host, &cfg, &input)?
        }
        // ... the remaining four tools, same shape
        other => return Err(RagError::InvalidInput(format!("unknown tool: {other}"))),
    };
    serde_json::to_string(&value).map_err(|e| RagError::Internal(format!("encode tool output: {e}")))
}
```

Note the order inside each arm: **parse arguments before resolving config.**
That order is forced here, not stylistic — the configuration is *inside* the
arguments, under the host-stamped `_tenant_overlay` key, so it cannot be read
until they are decoded. It is also what you want anyway: a malformed call should
fail the same way whether or not the extension is configured.

`lifecycle::Guest::init` is one line of real logic — parse, then store:

```rust
fn init(config_json: String) -> Result<(), types::ExtensionError> {
    let cfg = config::parse_config(&config_json).map_err(map_error)?;
    config::store(cfg).map_err(map_error)
}
```

And `map_error` is the one place `RagError` becomes the WIT
`extension-error`, closing the loop from step 6:

```rust
pub fn map_error(err: RagError) -> types::ExtensionError {
    match err {
        RagError::InvalidInput(m) => types::ExtensionError::InvalidInput(m),
        RagError::PermissionDenied(m) => types::ExtensionError::PermissionDenied(m),
        RagError::NotFound(m) => types::ExtensionError::NotFound(m),
        RagError::SchemaInvalid(m) => types::ExtensionError::SchemaInvalid(m),
        RagError::Internal(m) => types::ExtensionError::Internal(m),
    }
}
```

`tools::Guest::list_tools()` maps `tool_meta::all_tools()` straight into the
WIT `ToolDefinition` shape — one more reason `tool_meta.rs` has to be the
single source of truth rather than something reconstructed here.

## 14. Generate bindings, then run the fast loop

`src/bindings.rs` does not exist until you build once:

```
gtdx dev --once          # or: cargo component build
```

This is gitignored on purpose — it regenerates from `wit/world.wit` +
`wit/deps/` on every build — and a fresh clone's first `cargo test` fails
with `cannot find export in bindings` until one build has produced it. Do
that once, then the development loop is just:

```
cargo test
```

No WASM runtime, no live vector store, milliseconds. This is where nearly
all development actually happens — the point of the whole pure/host
boundary from step 0.

## 15. Declare the manifest

`describe.json` is what the designer and the store read; it has to agree
with the code rather than describe it, because two things in it are checked
against the code by tests, not just eyeballed. `runtime.permissions` is the
capability gate the running extension is held to:

```json
"permissions": {
  "network": ["https://*.qdrant.io/*", "https://api.openai.com/*"],
  "secrets": [
    "secret://rag-qdrant/qdrant_api_key",
    "secret://rag-qdrant/embedding_api_key"
  ],
  "callExtensionKinds": [],
  "ui": { "fetchHosts": [], "platformApi": [] }
}
```

Both network hosts trace straight back to steps 4 and 10: the vector store's
host and the embeddings API's host are both required, or `http::fetch`
refuses the request before it leaves the sandbox — see [README.md §
Requirements and limits](../README.md#requirements-and-limits) for the
self-hosted-Qdrant and non-OpenAI-embeddings-host caveats. Both secrets
trace back to the same two steps, and `requiredSecrets` names them again
with a human-readable description for the operator configuring them.

`contributions.tools` is generated *from* `src/tool_meta.rs`, not
hand-written, and a host test
(`tool_meta::tests::describe_json_matches_the_tool_metadata_in_this_file`)
asserts the two never drift — comparing parsed JSON, not raw strings, so
reordering keys never causes a false failure. Regenerate it after any schema
change:

```
RUNTIME_REF=<runtime_ref> cargo test print_contributions -- --ignored --nocapture
```

and paste the printed JSON block into `describe.json`'s `contributions.tools`
by hand — this is a generator, not a check, and nothing runs it for you.

If your extension takes operator configuration, also declare a top-level
`configSchema` — a JSON Schema, as a string — naming the fields an admin
console can turn into a labelled form instead of a raw JSON editor. This
repo's is worth copying as a template: it names only `qdrant_url`,
`collection` and `require_tenant_overlay`, the flat fields with no working
default or a real default worth exposing, and deliberately leaves out
`embedding`/`chunk` because the renderer this schema targets falls through
to raw JSON for nested objects — declaring one would promise a form control
it cannot produce. `configSchema` is a manifest field newer than some `gtdx`
releases understand: `DescribeJson` denies unknown fields, so an old `gtdx`
rejects the whole describe outright rather than ignoring the one field it
doesn't recognise. Pin your CI's `gtdx` version accordingly (see this repo's
`.github/workflows/check.yml`).

Validate after every edit:

```
gtdx validate && gtdx lint
```

`gtdx validate` checks JSON Schema shape; `gtdx lint` checks cross-field
invariants (id pattern, schema host, remote-asset rules for a contributed
view, …). Both are cheap and catch a broken manifest that compiles fine but
the runtime or store will reject.

## 16. Run the full gate

```
./ci/local_check.sh
```

In order: `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D
warnings`, `cargo test`, `./build.sh` (`cargo component build --release`).
The bar is zero clippy warnings and a fully green suite.

One thing to know about this script rather than be confused by later: its
own last step regenerates the gitignored `src/bindings.rs`, unformatted,
which then fails the *first* gate (`cargo fmt --all -- --check`) the next
time you run the script. This is a known scaffold issue, not specific to
this extension — run `cargo fmt` before your next commit and move on; it
does not mean the previous run's pass was invalid.

## 17. Install and publish

```
gtdx dev             # watch: rebuild + reinstall to the local registry on save
gtdx dev --once       # do it once — proves the package installs, not that it behaves
gtdx doctor           # environment: cargo, cargo-component, wasm32-wasip2 target
gtdx lint --publish   # additionally rejects the placeholder 0000... sha256 (E_SHA256_ZERO)
gtdx publish          # build, compute real content hashes, pack the .gtxpack, install locally
gtdx verify           # verify the signature once published/signed
```

`gtdx publish` writes the `sha256` fields but never touches
`runtime.components.*.gtpack.component_version` — bump that by hand,
together with `Cargo.toml`'s `[package].version` and `describe.json`'s
`metadata.version`, on every release, or it silently drifts from the real
package.

## What to change for a different vector store

Everything in steps 6–9, 12–14 is store-agnostic: chunking, config shape
minus the store's own URL field, the error taxonomy, either/or input
enforcement, tool schemas and agentic-worker metadata, and the WIT glue all
look the same regardless of which store sits behind `ops.rs`.

**`src/qdrant.rs` is the only module that is actually Qdrant-specific** — its
request/response shapes, its REST paths, and its `parse_ensure_ack`'s
"already exists" 4xx handling are all particular to Qdrant's API. Swap it
for a module with the same shape (request builders in, parsed results out,
all pure, all taking a host reference for nothing — HTTP clients don't need
one, only the caller does) against your store's API instead.

`src/ops.rs` needs the least conceptual change but the most careful
line-by-line one: the *ordering* rules from step 11 — delete before upsert,
embed before delete — apply to any store, but the specific request shapes it
calls into `qdrant.rs` for change to whatever your new module exposes. The
point-id constraint in step 9 (`validate_point_id`) is Qdrant's own rule
(unsigned integer or UUID); check your store's documentation for its
equivalent and validate that instead — the *reason* to validate before any
host call (an unparseable id shouldn't cost an embeddings request first)
carries over even when the specific rule doesn't.

`src/embed.rs` doesn't change at all when you change vector stores — it
talks to the embeddings API, not the store — but it *does* change if you
swap embedding providers for one that isn't OpenAI-shaped.
