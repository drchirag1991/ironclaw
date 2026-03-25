#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

echo "Building Weixin channel WASM component..."

cargo build --release --target wasm32-wasip2

WASM_PATH="target/wasm32-wasip2/release/weixin_channel.wasm"

if [ -f "$WASM_PATH" ]; then
    wasm-tools component new "$WASM_PATH" -o weixin.wasm 2>/dev/null || cp "$WASM_PATH" weixin.wasm
    wasm-tools strip weixin.wasm -o weixin.wasm

    echo "Built: weixin.wasm ($(du -h weixin.wasm | cut -f1))"
    echo ""
    echo "To install:"
    echo "  mkdir -p ~/.ironclaw/channels"
    echo "  cp weixin.wasm weixin.capabilities.json ~/.ironclaw/channels/"
else
    echo "Error: WASM output not found at $WASM_PATH"
    exit 1
fi
