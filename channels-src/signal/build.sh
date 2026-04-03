#!/usr/bin/env bash
# Build the Signal channel WASM component
#
# Prerequisites:
#   - Rust with wasm32-wasip2 target: rustup target add wasm32-wasip2
#   - wasm-tools for component creation: cargo install wasm-tools
#
# Output:
#   - signal.wasm - WASM component ready for deployment
#   - signal.capabilities.json - Capabilities file (copy alongside .wasm)

set -euo pipefail

cd "$(dirname "$0")"

echo "Building Signal channel WASM component..."

# Build the WASM module (without default features to skip rayon + pqcrypto-kyber)
cargo build --release --target wasm32-wasip2

# Convert to component model
WASM_PATH="target/wasm32-wasip2/release/signal_channel.wasm"

if [ -f "$WASM_PATH" ]; then
    wasm-tools component new "$WASM_PATH" -o signal.wasm 2>/dev/null || cp "$WASM_PATH" signal.wasm
    wasm-tools strip signal.wasm -o signal.wasm

    echo "Built: signal.wasm ($(du -h signal.wasm | cut -f1))"
    echo ""
    echo "To install:"
    echo "  mkdir -p ~/.ironclaw/channels"
    echo "  cp signal.wasm signal.capabilities.json ~/.ironclaw/channels/"
else
    echo "Error: WASM output not found at $WASM_PATH"
    exit 1
fi
