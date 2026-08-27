#!/usr/bin/env bash
set -euo pipefail
# cargo-component 0.21 defaults to wasm32-wasip1 when --target is omitted,
# regardless of the single wasm32-wasip2 target declared in
# rust-toolchain.toml (that file controls which targets rustup installs,
# not which one cargo-component builds for). Pass --target explicitly so
# the output always lands where the rest of this script — and gtdx — expect
# it: target/wasm32-wasip2/release.
cargo component build --release --target wasm32-wasip2
mkdir -p dist
cd target/wasm32-wasip2/release
# Additional packaging done by `gtdx publish`; this script just builds the wasm.
ls -lh *.wasm
