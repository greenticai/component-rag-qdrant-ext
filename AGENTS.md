# AGENTS.md

Guidance for AI coding agents (Claude Code, Codex, and similar) working in this
repository. This is a Greentic Designer **design** extension scaffolded by
`gtdx new` (SDK 1.2.9, contract `greentic:extension-design@0.3.0`).

> Claude Code reads `CLAUDE.md`, which points back here. Keep **this** file as the
> single source of truth for both tools and update it as the extension grows.

## What this is

- **id:** `greentic.rag-qdrant`  ·  **version:** `0.1.0`  ·  **kind:** `design`
- A WebAssembly component (target `wasm32-wasip2`) that the Greentic runtime loads
  as a signed `.gtxpack`.
- `describe.json` is the manifest and the single source of truth for metadata —
  the runtime and store read it.

## Layout

- `describe.json`        — extension manifest (id, version, capabilities, runtime)
- `src/lib.rs`           — WASM guest exports (the code you change)
- `wit/`                 — WIT contract, vendored by `gtdx new` (see `.gtdx-contract.lock`)
- `i18n/en.json`         — user-facing strings
- `build.sh`             — compile the wasm
- `ci/local_check.sh`    — fmt + clippy + test + build gate
- `.claude/`             — Claude Code config: pre-approved build perms + `/check` command

## Workflow

```
gtdx dev        # watch: rebuild + reinstall to the local registry on save
gtdx publish    # build, pack to dist/greentic.rag-qdrant-0.1.0.gtxpack, install locally
./build.sh      # just compile the wasm (cargo component build --release)
./ci/local_check.sh   # fmt + clippy -D warnings + test + build
```

Run `./ci/local_check.sh` before every commit and before publishing. The bar is
**zero clippy warnings** (`cargo clippy --all-targets -- -D warnings`) and green
tests. Do not commit if it fails.

## Testing

Three layers, fastest first. Most of your testing belongs in the first one.

| Layer | Command | What it proves |
|---|---|---|
| Unit, on the host | `cargo test` | Your logic is right. Milliseconds, no WASM, no designer. |
| Full gate | `./ci/local_check.sh` | fmt + clippy + tests + the wasm actually builds. |
| Integration | `gtdx dev --once` | It packs and installs. Not that it behaves — that is the layer above. |

The guest exports are plain Rust functions, so a host test calls them directly:

```rust
let out = <Component as tools::Guest>::invoke_tool("echo".into(), args)?;
```

`src/lib.rs` ships example tests doing exactly this. Extend them; do not delete
them and leave `cargo test` with nothing to run.

**One prerequisite:** `cargo test` needs `src/bindings.rs`, which is generated
rather than committed. Build once (`gtdx dev --once` or `cargo component build`)
before the first `cargo test` in a fresh clone, or it fails with
`cannot find export in bindings`.

Testing code that calls host functions — http, secrets, state, logging,
translation? Add the SDK's mocks as a dev-dependency:

```toml
[dev-dependencies]
greentic-extension-sdk-testing = "1.2.9"
```

They are ordinary in-memory objects (`MockHttpClient`, `MockSecretsBackend`,
`MockLogger`, …), so they only help if your code takes the host dependency as a
parameter rather than calling the binding directly. Structure it that way and
the logic stays testable on the host.

## Self-check before publishing

Beyond the Rust gate above, validate the manifest with gtdx — these catch a broken
`describe.json` that compiles fine but the runtime/store will reject:

```
gtdx doctor     # environment: cargo, cargo-component, wasm32-wasip2 target
gtdx validate   # describe.json against the JSON Schema
gtdx lint       # describe.json cross-field invariants (id pattern, schema host, …)
gtdx lint --publish   # also rejects placeholder 0000… sha256 (E_SHA256_ZERO)
gtdx verify     # verify the signature once published/signed
```

Run `gtdx validate` and `gtdx lint` after any edit to `describe.json`, and
`gtdx lint --publish` right before `gtdx publish`.

## Replace these — they are scaffold/sample values

- **id namespace** — if the id is still the `com.example.*` reverse-DNS sample,
  change it to your own namespace. The id (`greentic.rag-qdrant`) and its WIT-package form
  (`greentic:rag-qdrant`) must stay in sync everywhere they appear:
  - `describe.json` (`metadata.id`, and any `runtime.world` / `runtime.components` key)
  - `Cargo.toml` (`package.metadata.component.package`)
  - `wit/world.wit` (the `package` line)
- **metadata** — `name`, `summary`, `description`, and `author` in `describe.json`.
- **the example export** — `src/lib.rs` ships one placeholder tool/node that you
  are meant to replace with the real surface your extension exposes. **Read the
  doc-comments at the top of `src/lib.rs`** — they describe the per-kind pattern
  for this extension (which functions to add to, and how the host calls them).
- **i18n** — the strings in `i18n/en.json`.

## Secrets

Never hardcode credentials or API keys in `src/lib.rs`. If the extension needs a
secret, declare it under `secret_requirements` in `describe.json`; the runtime
resolves and injects it at execution time.

## Do NOT hand-edit these — generated or managed

- **`sha256` fields in `describe.json`** — the `0000…0000` values are placeholders.
  `gtdx publish` computes the real content hashes. Never fill them in by hand.
- **`.gtdx-contract.lock`** — generated by `gtdx new`; pins the vendored WIT
  contract hashes. Don't edit.
- **`wit/deps/`** — vendored WIT contract. Treat as read-only; it is locked.
- **`src/bindings.rs`** — generated bindings (gitignored). Never commit or edit.
- **`target/`, `dist/`, `*.gtxpack`** — build output (gitignored).

## Conventions

- Rust edition 2024; toolchain pinned in `rust-toolchain.toml` — don't float it.
- Build target is `wasm32-wasip2` only.
- `describe.json` must stay valid v2 shape: `apiVersion: greentic.ai/v2`, a `compat`
  block, and a `runtime.components` map.
- JSON for `describe.json`, YAML for human-authored configs, CBOR for binary metadata.
