#!/usr/bin/env bash

set -eo pipefail

# build runtime
WASM_BUILD_NO_COLOR=1 cargo build --verbose --release -p chainx-runtime
# make checksum
sha256sum target/release/wbuild/target/wasm32-unknown-unknown/release/chainx_runtime.wasm > checksum.sha256
# clean up – FIXME: can we reuse some of the artifacts?
cargo clean
# build again
WASM_BUILD_NO_COLOR=1 cargo build --verbose --release -p chainx-runtime
# confirm checksum
sha256sum -c checksum.sha256
