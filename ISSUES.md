# Railway Deployment Issues

## Issue 1: Building from Source Times Out

**Problem:** Building Rust from source on Railway times out (Railway has ~15 min build limit).

**Solution:** Use pre-built binary instead of compiling from source.

## Issue 2: Binary Path Not in PATH

**Problem:** The installer places the binary in `/root/.cargo/bin/` but Dockerfile used `/root/.local/bin/`.

**Solution:** Set `ENV PATH="/root/.cargo/bin:$PATH"` in Dockerfile.

## Issue 3: Interactive Onboarding Blocks Startup

**Problem:** IronClaw's onboarding wizard prompts for input, blocking startup in non-interactive environments.

**Solution:** 
- Set `ONBOARD_COMPLETED=true` to skip onboarding
- Use empty pre-created database to bypass database check

## Issue 4: Database Persistence

**Problem:** Deleting DB on every restart causes onboarding to run again.

**Solution:** Create an empty DB file in Dockerfile so migrations run but onboarding is skipped:
```dockerfile
RUN mkdir -p /root/.ironclaw && touch /root/.ironclaw/ironclaw.db
```

## Issue 5: Model Override Env Var

**Problem:** `LLM_MODEL` env var is not used by OpenRouter provider. It uses `OPENROUTER_MODEL` instead.

**Solution:** Set `OPENROUTER_MODEL` not `LLM_MODEL` for OpenRouter backend.

**Reference:** In `providers.json`, openrouter provider defines:
```json
"model_env": "OPENROUTER_MODEL",
"default_model": "openai/gpt-4o"
```

## Issue 6: Healthcheck Fails

**Problem:** Railway healthcheck fails if the app doesn't respond to HTTP requests immediately.

**Solution:** Remove healthcheckPath from railway.json or set it appropriately.

## Issue 7: Gateway Not Accessible Externally

**Problem:** Gateway binds to 127.0.0.1, only accessible inside container.

**Solution:** Set `GATEWAY_HOST=0.0.0.0` to bind to all interfaces.

## Issue 8: DB Schema Not Initialized

**Problem:** Empty DB file causes migration failures.

****Solution:** Let IronClaw create and migrate the DB on first run. Use `touch` to create empty file, IronClaw will apply migrations.

## Issue 9: Port Incompatibility

**Problem:** Railway dynamically assigns a `PORT` environment variable and expects the container to listen on it. IronClaw defaults to `3000` for its web gateway; external routing fails if the container is not explicitly configured to listen on 3000 via a standard `PORT` var.

**Solution:** Set `PORT=3000` in Railway environment variables and ensure the container's gateway configuration matches this.

## Issue 10: Sandbox Execution Failures

**Problem:** Standard Railway containers do not have Docker installed, leading to failures in sandbox-dependent routines (`full_job`).

**Solution:** Disable sandbox tasks or use a custom Dockerfile with Docker-in-Docker (DinD) support if sandboxing is required.

## Working Dockerfile

```dockerfile
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y curl && rm -rf /var/lib/apt/lists/*

RUN curl -fsSL https://github.com/nearai/ironclaw/releases/latest/download/ironclaw-installer.sh | sh

ENV PATH="/root/.cargo/bin:$PATH"

RUN mkdir -p /root/.ironclaw && touch /root/.ironclaw/ironclaw.db

EXPOSE 3000

CMD ["ironclaw", "--no-onboard"]
```

## Required Environment Variables

| Variable | Value | Notes |
|----------|-------|-------|
| DATABASE_BACKEND | libsql | Use embedded DB |
| LLM_BACKEND | openrouter | OpenRouter provider |
| OPENROUTER_API_KEY | sk-or-v1-... | From openrouter.ai |
| OPENROUTER_MODEL | qwen/qwen3.6-plus:free | Free model |
| SECRETS_MASTER_KEY | random-string | For encryption |
| GATEWAY_ENABLED | true | Enable web UI |
| GATEWAY_HOST | 0.0.0.0 | Bind to all interfaces |
| ONBOARD_COMPLETED | true | Skip onboarding wizard |
| PORT | 3000 | Required for Railway routing |
