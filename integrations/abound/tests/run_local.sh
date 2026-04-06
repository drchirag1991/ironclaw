#!/bin/bash
# Run Abound E2E tests against a local IronClaw server.
#
# Usage:
#   bash integrations/abound/tests/run_local.sh
#
# Prerequisites:
#   - cargo build (or let the script build for you)
#   - NearAI session token at ~/.ironclaw/session.json (run `ironclaw onboard`)
#   - Or set ANTHROPIC_API_KEY / LLM_BACKEND env vars before running

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
LOCAL_PORT=3199
LOCAL_URL="http://127.0.0.1:${LOCAL_PORT}"
LOCAL_DB="/tmp/ironclaw_e2e_test_$$.db"

echo "=== Building IronClaw ==="
cd "$ROOT_DIR"
cargo build --release 2>&1 | tail -3

echo ""
echo "=== Starting local server on port ${LOCAL_PORT} ==="

# Start server with Abound integration config
RUST_LOG=ironclaw=info \
DATABASE_BACKEND=libsql \
LIBSQL_PATH="$LOCAL_DB" \
ONBOARD_COMPLETED=true \
GATEWAY_AUTH_TOKEN=localtest \
GATEWAY_PORT="$LOCAL_PORT" \
AGENT_AUTO_APPROVE_TOOLS=true \
INTEGRATION_CREDENTIALS_DIR="$ROOT_DIR/integrations" \
AGENTS_SEED_PATH="$ROOT_DIR/integrations/abound/workspace/AGENTS.md" \
SKILLS_DIR="$ROOT_DIR/skills" \
NEARAI_MODEL="${NEARAI_MODEL:-anthropic/claude-sonnet-4-5}" \
  target/release/ironclaw &

SERVER_PID=$!

cleanup() {
    echo ""
    echo "=== Stopping server (PID $SERVER_PID) ==="
    kill $SERVER_PID 2>/dev/null
    wait $SERVER_PID 2>/dev/null
    rm -f "$LOCAL_DB" "$LOCAL_DB-wal" "$LOCAL_DB-shm"
    echo "Done"
}
trap cleanup EXIT

# Wait for server to be ready
echo "Waiting for server..."
for i in $(seq 1 30); do
    if curl -s "$LOCAL_URL/" >/dev/null 2>&1; then
        echo "Server ready"
        break
    fi
    if ! kill -0 $SERVER_PID 2>/dev/null; then
        echo "Server failed to start"
        exit 1
    fi
    sleep 1
done

echo ""
echo "=== Running E2E tests ==="

export BASE_URL="$LOCAL_URL"
export ADMIN_TOKEN=localtest
export ABOUND_BEARER_TOKEN=eyJhbGciOiJIUzM4NCJ9.eyJleHAiOjE3Nzc4MzA1MTEsImN1c3RvbWVyX2lkIjoiMThkYjQ4MjktODdhZS00YzhhLTlmNTYtNDNhZjU0NmVmZGVlLTE3NTQzNzYxNDEwMjMifQ.zyTZqyeDgn-YCF0qhsB5LfpFfKUsTx-TygOEW85wmtGvdZtyfLtDkfY1j1q5ndmR
export ABOUND_API_KEY=a105acd4-74f6-46b6-b429-c2b764462b99
export ABOUND_WRITE_TOKEN=eyJhbGciOiJIUzI1NiJ9.eyJleHAiOjE3NzU2NzA0ODIsImN1c3RvbWVyX2lkIjoiMThkYjQ4MjktODdhZS00YzhhLTlmNTYtNDNhZjU0NmVmZGVlLTE3NTQzNzYxNDEwMjMifQ.pkS5IqOSCmxNvH3qjicUY2UQa82wKtCpLNBoHGAfGgg
export MASSIVE_API_KEY=mjKhxGjLq04FVUzaOtgW_iE0rSToKhC0

uv run python integrations/abound/tests/test_abound_e2e.py
