#!/usr/bin/env bash
set -euo pipefail
cargo component build --release
mkdir -p dist
cd target/wasm32-wasip2/release
# Additional packaging done by `gtdx publish`; this script just builds the wasm.
ls -lh *.wasm
