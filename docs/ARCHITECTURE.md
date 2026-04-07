# IronClaw Architecture

Detailed architecture reference covering deployment models, the tool ecosystem, and the agent loop.

## Table of Contents

- [1. Single-Tenant Setup](#1-single-tenant-setup)
- [2. Multi-Tenant Setup](#2-multi-tenant-setup)
- [3. Tools / Extensions / Skills / CodeAct](#3-tools--extensions--skills--codeact)
- [4. Agent Loop](#4-agent-loop)

---

## 1. Single-Tenant Setup

The default deployment model — one IronClaw process serving one user.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        SINGLE-TENANT DEPLOYMENT                             │
│                                                                             │
│  ~/.ironclaw/                                                               │
│  ├── .env                    ← bootstrap config (DATABASE_URL, LLM_BACKEND) │
│  ├── ironclaw.pid            ← PID lock (one process only)                  │
│  ├── settings.json           ← user preferences                            │
│  ├── ironclaw.db             ← libSQL local DB (if libsql backend)          │
│  ├── channels/               ← WASM channel binaries (.wasm)               │
│  ├── tools/                  ← installed WASM tools                        │
│  ├── skills/                 ← user SKILL.md files (trusted)               │
│  ├── installed_skills/       ← registry SKILL.md files (installed trust)   │
│  ├── mcp-servers.json        ← MCP server configs                          │
│  └── projects/               ← bind-mount paths for Docker sandbox          │
│                                                                             │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                       IronClaw Process                               │   │
│  │                       (owner_id: "henry")                            │   │
│  │                                                                      │   │
│  │   ┌──────────────── Channels (all emit IncomingMessage) ──────────┐  │   │
│  │   │                                                               │  │   │
│  │   │  ┌──────┐  ┌──────────┐  ┌───────────┐  ┌───────────────┐    │  │   │
│  │   │  │ TUI  │  │ HTTP     │  │ WASM      │  │ Web Gateway   │    │  │   │
│  │   │  │(REPL)│  │ Webhook  │  │ Channels  │  │ (Browser UI)  │    │  │   │
│  │   │  │      │  │          │  │ Telegram,  │  │ SSE + WS      │    │  │   │
│  │   │  │stdin │  │ POST /wh │  │ Discord,  │  │ REST API      │    │  │   │
│  │   │  │→owner│  │ →owner   │  │ Slack...  │  │ Single bearer │    │  │   │
│  │   │  └──┬───┘  └────┬─────┘  │→pairing   │  │ token →owner  │    │  │   │
│  │   │     │           │        └─────┬─────┘  └──────┬────────┘    │  │   │
│  │   └─────┴───────────┴──────────────┴───────────────┘             │  │   │
│  │                              │                                    │  │   │
│  │                    ┌─────────▼──────────┐                         │  │   │
│  │                    │  ChannelManager    │                         │  │   │
│  │                    │  (merged stream)   │                         │  │   │
│  │                    └─────────┬──────────┘                         │  │   │
│  │                              │                                    │  │   │
│  │                    ┌─────────▼──────────┐                         │  │   │
│  │                    │    Agent Loop      │  1 Session for owner    │  │   │
│  │                    │  (SessionManager)  │  N Threads per channel  │  │   │
│  │                    └────┬─────────┬─────┘                         │  │   │
│  │                         │         │                               │  │   │
│  │              ┌──────────▼───┐  ┌──▼────────────┐                  │  │   │
│  │              │  Scheduler   │  │ Routine Engine │                  │  │   │
│  │              │ (parallel    │  │ (cron, event,  │                  │  │   │
│  │              │  jobs)       │  │  webhook)      │                  │  │   │
│  │              └──────┬───────┘  └────────┬───────┘                  │  │   │
│  │                     │                   │                          │  │   │
│  │     ┌───────────────┼───────────────────┘                          │  │   │
│  │     │               │                                              │  │   │
│  │  ┌──▼──────┐  ┌─────▼──────────────┐                               │  │   │
│  │  │ Local   │  │   Orchestrator     │  port 50051                   │  │   │
│  │  │ Workers │  │  ┌───────────────┐ │                               │  │   │
│  │  │(in-proc)│  │  │Docker Sandbox │ │                               │  │   │
│  │  │         │  │  │ ┌───────────┐ │ │                               │  │   │
│  │  │ChatDel. │  │  │ │Worker    ││ │                               │  │   │
│  │  │JobDel.  │  │  │ │Container ││ │                               │  │   │
│  │  └────┬────┘  │  │ │Delegate  ││ │                               │  │   │
│  │       │       │  │ └───────────┘ │ │                               │  │   │
│  │       │       │  └───────────────┘ │                               │  │   │
│  │       │       └─────────┬──────────┘                               │  │   │
│  │       └─────────────────┤                                          │  │   │
│  │                         │                                          │  │   │
│  │              ┌──────────▼───────────┐                               │  │   │
│  │              │    Tool Registry     │                               │  │   │
│  │              │ Built-in│MCP│WASM    │                               │  │   │
│  │              └──────────────────────┘                               │  │   │
│  │                         │                                          │  │   │
│  │              ┌──────────▼───────────┐                               │  │   │
│  │              │  LLM Provider Chain  │                               │  │   │
│  │              │ Retry→SmartRoute→    │                               │  │   │
│  │              │ Failover→CB→Cache    │                               │  │   │
│  │              └──────────────────────┘                               │  │   │
│  │                         │                                          │  │   │
│  │              ┌──────────▼───────────┐                               │  │   │
│  │              │    Database          │                               │  │   │
│  │              │ PostgreSQL or libSQL │                               │  │   │
│  │              │ (all rows: user_id=  │                               │  │   │
│  │              │  owner_id)           │                               │  │   │
│  │              └──────────────────────┘                               │  │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Key characteristics

- **PID lock** prevents multiple instances (`~/.ironclaw/ironclaw.pid`)
- **Single `owner_id`** (e.g. `"henry"`) — all DB rows, sessions, workspaces scoped to this
- **One Session** in `SessionManager`, with one Thread per (channel, conversation)
- **Database**: PostgreSQL (pool-based, `deadpool-postgres`) or libSQL/Turso (zero infrastructure, `~/.ironclaw/ironclaw.db`)
- **Web Gateway auth**: single `GATEWAY_AUTH_TOKEN` via `MultiAuthState::single(token, owner_id)`
- **Docker sandbox**: optional — TOCTOU assumptions acceptable in single-tenant ("user controls the filesystem")

### AppBuilder 5-phase init (`src/app.rs`)

1. **Database** — connect PostgreSQL or libSQL, run migrations, bootstrap owner user row, reload config from DB
2. **Secrets** — create `SecretsStore` (AES-256-GCM), migrate plaintext API keys into encrypted store
3. **LLM** — build provider chain with retry, smart routing, failover, circuit breaker, response cache
4. **Safety, Tools, Workspace** — create `SafetyLayer`, `ToolRegistry`, embeddings provider, `Workspace`, `WorkspacePool`
5. **Extensions** — load WASM tools, connect MCP servers, create `ExtensionManager`, wire into `AppComponents`

### Session / Thread / Turn hierarchy

```
Session (owner_id = "henry")          <- exactly one
├── Thread (REPL, default)            <- per channel + conversation
│   ├── Turn 1: user_input -> response + tool_calls
│   ├── Turn 2: ...
│   └── Turn N: ...
├── Thread (Telegram, chat_id=123)
├── Thread (Web Gateway, conv_abc)
└── Thread (Web Gateway, conv_def)
```

- Sessions pruned after 10 minutes idle (warn at 1,000 sessions)
- Turns are append-only; undo restores prior checkpoint (max 20 per thread)
- Group chats: `MEMORY.md` excluded from system prompt to prevent leaking personal context
- Auth mode: if thread has `pending_auth`, next message bypasses LLM/hooks and goes directly to credential store

---

## 2. Multi-Tenant Setup

Shared server serving multiple users — same binary, shared database, row-level isolation.

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                        MULTI-TENANT DEPLOYMENT                               │
│                                                                              │
│              ┌──────────────────────────────────────────────┐                │
│              │           External Users                     │                │
│              │                                              │                │
│              │  User A          User B          User C      │                │
│              │  (Telegram)      (Browser)       (Browser)   │                │
│              └──────┬──────────────┬──────────────┬─────────┘                │
│                     │              │              │                           │
│  ┌──────────────────┼──────────────┼──────────────┼────────────────────────┐  │
│  │                  │    IronClaw Process         │                        │  │
│  │                  │    (owner_id: "admin")      │                        │  │
│  │                  │                             │                        │  │
│  │  ┌───────────────▼──────────────▼──────────────▼──────────────────────┐ │  │
│  │  │                     Auth Layer                                    │ │  │
│  │  │                                                                   │ │  │
│  │  │  WASM Channels:              Web Gateway:                         │ │  │
│  │  │  ┌─────────────────┐         ┌────────────────────────────────┐   │ │  │
│  │  │  │ PairingStore    │         │ CombinedAuthState              │   │ │  │
│  │  │  │                 │         │                                │   │ │  │
│  │  │  │ (channel,       │         │ 1. Bearer token (env var)     │   │ │  │
│  │  │  │  sender_id)     │         │ 2. DbAuthenticator            │   │ │  │
│  │  │  │  -> Identity    │         │    (LRU cache, TTL 60s)       │   │ │  │
│  │  │  │                 │         │ 3. OIDC JWT (optional)        │   │ │  │
│  │  │  │ Pending ->      │         │    (AWS ALB / Okta / Cognito) │   │ │  │
│  │  │  │ approve via     │         │                                │   │ │  │
│  │  │  │ CLI or web UI   │         │ -> AuthenticatedUser {         │   │ │  │
│  │  │  │                 │         │     user_id, role, scopes }    │   │ │  │
│  │  │  └─────────────────┘         └────────────────────────────────┘   │ │  │
│  │  └───────────────────────────────────────────────────────────────────┘ │  │
│  │                                    │                                   │  │
│  │                    ┌───────────────▼────────────────┐                   │  │
│  │                    │      Per-User Isolation        │                   │  │
│  │                    │                                │                   │  │
│  │                    │  TenantScope (compile-time)    │                   │  │
│  │                    │    All DB ops bound to user_id │                   │  │
│  │                    │    get_job(id) -> None if not  │                   │  │
│  │                    │    owned. Lists filter by user │                   │  │
│  │                    │                                │                   │  │
│  │                    │  WorkspacePool (lazy cache)    │                   │  │
│  │                    │    User A -> Workspace A       │                   │  │
│  │                    │    User B -> Workspace B       │                   │  │
│  │                    │    (seeded on first access)    │                   │  │
│  │                    │                                │                   │  │
│  │                    │  PerUserRateLimiter            │                   │  │
│  │                    │    LRU 2048 users              │                   │  │
│  │                    │    30 msg/60s per user         │                   │  │
│  │                    │                                │                   │  │
│  │                    │  SSE per-user scoping          │                   │  │
│  │                    │    broadcast_for_user(user_id) │                   │  │
│  │                    └────────────────────────────────┘                   │  │
│  │                                    │                                   │  │
│  │                    ┌───────────────▼────────────────┐                   │  │
│  │                    │     SessionManager             │                   │  │
│  │                    │                                │                   │  │
│  │                    │  User A -> Session A           │                   │  │
│  │                    │    ├── Thread (Telegram, dm)   │                   │  │
│  │                    │    └── Thread (Telegram, grp)  │                   │  │
│  │                    │                                │                   │  │
│  │                    │  User B -> Session B           │                   │  │
│  │                    │    └── Thread (web, conv_1)    │                   │  │
│  │                    │                                │                   │  │
│  │                    │  User C -> Session C           │                   │  │
│  │                    │    ├── Thread (web, conv_2)    │                   │  │
│  │                    │    └── Thread (web, conv_3)    │                   │  │
│  │                    └────────────────────────────────┘                   │  │
│  │                                                                        │  │
│  │  Admin API (requires UserRole::Admin):                                 │  │
│  │    POST   /api/admin/users          <- create user + initial token     │  │
│  │    GET    /api/admin/users          <- list users + LLM usage stats    │  │
│  │    PATCH  /api/admin/users/{id}     <- update profile                  │  │
│  │    DELETE /api/admin/users/{id}     <- delete user and all data        │  │
│  │    POST   /api/admin/users/{id}/suspend|activate                       │  │
│  │    GET    /api/admin/usage          <- per-user LLM usage              │  │
│  └────────────────────────────────────────────────────────────────────────┘  │
│                                                                              │
│  ┌────────────────────────────────────────────────────────────────────────┐  │
│  │                    Shared Database (PostgreSQL)                         │  │
│  │                                                                        │  │
│  │  Every table has user_id column for row-level scoping:                 │  │
│  │                                                                        │  │
│  │  agent_jobs(user_id)         conversations(user_id)                    │  │
│  │  routines(user_id)           conversation_messages(->conversation)     │  │
│  │  settings(user_id, key)      memory_documents(user_id)                 │  │
│  │  secrets(user_id, name)      memory_chunks(user_id)                    │  │
│  └────────────────────────────────────────────────────────────────────────┘  │
│                                                                              │
│  Scope types:                                                                │
│    TenantScope  - per-request, bound to user_id (all handler code)          │
│    SystemScope  - cross-tenant (heartbeat, routine engine, self-repair)      │
│    AdminScope   - requires UserRole::Admin (admin API endpoints)            │
│                                                                              │
│  Known multi-tenant gaps (documented in src/ownership/mod.rs):              │
│    - Extension lifecycle/configuration remains owner-scoped                 │
│    - Orchestrator secret injection is owner-scoped                          │
│    - Some channel secret setup still owner-scoped                           │
│    - MCP session management still owner-scoped                              │
└──────────────────────────────────────────────────────────────────────────────┘
```

### Single-tenant vs multi-tenant comparison

| Dimension | Single-Tenant | Multi-Tenant |
|-----------|--------------|-------------|
| **Auth** | One `GATEWAY_AUTH_TOKEN` | `DbAuthenticator` + per-user tokens, optional OIDC |
| **Sessions** | 1 Session (owner) | N Sessions (one per `user_id`) |
| **Database** | All rows `user_id=owner_id` | Shared DB, `TenantScope` enforces row ownership |
| **Workspace** | Single `Workspace(owner_id)` | `WorkspacePool` lazily creates `Workspace(user_id)` per user |
| **Rate limiting** | Single limiter (still per-user keyed) | Independent sliding window per user |
| **SSE events** | `broadcast(event)` to all clients | `broadcast_for_user(user_id, event)` user-scoped |
| **Sandbox** | Containers run as owner | `job_owner_cache` maps job_id to user_id |
| **Pairing** | Owner approves channel users | Same mechanism, each approved user gets own user_id |
| **Deploy** | Local binary or daemon | Railway/Docker + PostgreSQL |

### Pairing system (WASM channel admission)

When an unknown sender messages the bot via a WASM channel (Telegram, etc.):

1. `PairingStore::upsert_request(channel, external_id, meta)` creates a pending request with a time-limited code
2. Owner approves via `ironclaw pairing approve <code>` (CLI) or `POST /api/pairing/{channel}/approve` (web)
3. Approval maps `(channel, external_id)` to a `user.id` row in `channel_identities`
4. `PairingStore::resolve_identity()` hot path: `OwnershipCache` first (zero DB reads on hit), then DB fallback

### Deployment (Railway)

The `railway.toml` configures a Dockerfile build with health check at `/api/health`. The container exposes port 3000 and runs as a non-root `ironclaw` user. Docker-in-Docker is not available on Railway, so sandbox/container orchestration is disabled (graceful degradation via `DockerStatus::NotInstalled`).

---

## 3. Tools / Extensions / Skills / CodeAct

### Ecosystem overview

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                      TOOL & CAPABILITY ECOSYSTEM                             │
│                                                                              │
│  ┌─── BUILT-IN (compiled into binary) ──────────────────────────────────┐   │
│  │                                                                      │   │
│  │  Core:    echo, time, json, http, restart, message, plan_update      │   │
│  │  Files:   read_file, write_file, list_dir, apply_patch, shell        │   │
│  │  Memory:  memory_search, memory_write, memory_read, memory_tree      │   │
│  │  Jobs:    create_job, list_jobs, job_status, job_events,             │   │
│  │           job_prompt, cancel_job                                     │   │
│  │  Routine: routine_create/list/update/delete/fire/history, event_emit │   │
│  │  Ext Mgmt: tool_search, tool_install, tool_auth, tool_activate,     │   │
│  │           tool_list, tool_remove, tool_upgrade, tool_info,           │   │
│  │           extension_info                                             │   │
│  │  Skills:  skill_list, skill_search, skill_install, skill_remove      │   │
│  │  Secrets: secret_list, secret_delete                                 │   │
│  │  Image:   image_generate, image_edit, image_analyze                  │   │
│  │  Perms:   tool_permission_set                                        │   │
│  │  Builder: build_software                                             │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│                                                                              │
│  ┌─── WASM TOOLS (sandboxed, downloadable) ─────────────────────────────┐   │
│  │                                                                      │   │
│  │  Runtime: Wasmtime component model, WIT bindings (wit/tool.wit)      │   │
│  │                                                                      │   │
│  │  Available in tools-src/ (build from source):                        │   │
│  │    github, gmail, google-calendar, google-docs, google-drive,        │   │
│  │    google-sheets, google-slides, llm-context, slack, telegram,       │   │
│  │    web-search                                                        │   │
│  │                                                                      │   │
│  │  Security pipeline:                                                  │   │
│  │    Allowlist ─> Leak Scan ─> Credential ─> Execute ─> Leak Scan     │   │
│  │    Validator    (request)    Injector      Request    (response)     │   │
│  │                                                                      │   │
│  │  Capabilities (all opt-in, deny-by-default):                         │   │
│  │    HttpCapability      - allowlisted endpoints, rate limits           │   │
│  │    WorkspaceCapability - allowed path prefixes                        │   │
│  │    ToolInvokeCapability - tool aliasing, rate limits                  │   │
│  │    SecretsCapability    - allowed secret names (existence only)       │   │
│  │    WebhookCapability    - auth + signature verification              │   │
│  │                                                                      │   │
│  │  Resource limits:                                                    │   │
│  │    Memory: 10 MB | Fuel: 10M instructions | Timeout: 60s            │   │
│  │    BLAKE3 hash verification on load | Fresh instance per execute()   │   │
│  │                                                                      │   │
│  │  Storage: DB-backed (PostgresWasmToolStore / LibSqlWasmToolStore)    │   │
│  │  Trust levels: System > Verified > User                              │   │
│  │  Status: Active / Disabled / Quarantined                             │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│                                                                              │
│  ┌─── MCP SERVERS (external, any language) ──────────────────────────────┐  │
│  │                                                                       │  │
│  │  Config: ~/.ironclaw/mcp-servers.json                                 │  │
│  │                                                                       │  │
│  │  Transports:                                                          │  │
│  │    Http     - HTTP / Streamable HTTP / SSE                            │  │
│  │    Stdio    - subprocess (command + args + env)                        │  │
│  │    Unix     - Unix domain socket                                      │  │
│  │                                                                       │  │
│  │  Auth: OAuth 2.1 with Dynamic Client Registration                     │  │
│  │  Session: McpSessionManager tracks per-server state                   │  │
│  │                                                                       │  │
│  │  Pre-configured in registry/mcp-servers/:                             │  │
│  │    asana, cloudflare, intercom, linear, nearai, notion, sentry,       │  │
│  │    stripe                                                             │  │
│  └───────────────────────────────────────────────────────────────────────┘  │
│                                                                              │
│  ┌─── EXTENSION REGISTRY (discovery + install) ──────────────────────────┐  │
│  │                                                                       │  │
│  │  Catalog: registry/{tools,channels,mcp-servers}/<name>/manifest.json  │  │
│  │  ExtensionKind: WasmTool | WasmChannel | McpServer | ChannelRelay |   │  │
│  │                 AcpAgent                                              │  │
│  │                                                                       │  │
│  │  Bundles (registry/_bundles.json):                                    │  │
│  │    "google"    - gmail, gcal, gdocs, gdrive, gsheets, gslides        │  │
│  │    "messaging" - discord, telegram, slack, whatsapp, feishu           │  │
│  │    "default"   - github, gmail, gcal, gdrive, slack, telegram         │  │
│  │                                                                       │  │
│  │  Install flow:                                                        │  │
│  │    tool_search -> tool_install -> tool_auth -> tool_activate          │  │
│  │                                                                       │  │
│  │  Installer: download pre-built .wasm (SHA256 verified) or build       │  │
│  │             from source via cargo component build                     │  │
│  └───────────────────────────────────────────────────────────────────────┘  │
│                                                                              │
│  ┌─── SKILLS (prompt extensions, no code execution) ─────────────────────┐  │
│  │                                                                       │  │
│  │  Format: SKILL.md files (YAML frontmatter + markdown prompt body)     │  │
│  │                                                                       │  │
│  │  Trust model:                                                         │  │
│  │    TRUSTED   - ~/.ironclaw/skills/ or workspace skills/               │  │
│  │               (full tool access)                                      │  │
│  │    INSTALLED - ~/.ironclaw/installed_skills/ (from registry)          │  │
│  │               (read-only tools only: memory_search, memory_read,      │  │
│  │                memory_tree, time, echo, json, skill_list,             │  │
│  │                skill_search)                                          │  │
│  │                                                                       │  │
│  │  Selection pipeline (deterministic, no LLM call):                     │  │
│  │    1. Gating    - check bins/env/config requirements                  │  │
│  │    2. Scoring   - keywords (+10/+5), tags (+3), regex patterns (+20)  │  │
│  │    3. Budget    - fit top skills within MAX_SKILL_CONTEXT_TOKENS (4K) │  │
│  │    4. Attenuate - min trust of active skills sets tool ceiling         │  │
│  │                                                                       │  │
│  │  Explicit activation: /skill-name in message force-activates it       │  │
│  │                                                                       │  │
│  │  Bundled: delegation, github, ironclaw-workflow-orchestrator, linear,  │  │
│  │    local-test, plan-mode, review-checklist, routine-advisor,          │  │
│  │    web-ui-test                                                        │  │
│  │                                                                       │  │
│  │  Injection: <skill name="X" version="Y" trust="TRUSTED">...</skill>  │  │
│  │  appended to system prompt before LLM call                            │  │
│  └───────────────────────────────────────────────────────────────────────┘  │
│                                                                              │
│  ┌─── CodeAct / ENGINE V2 (Tier 0 + Tier 1 execution) ──────────────────┐  │
│  │                                                                       │  │
│  │  Activated by: config.engine_v2 = true                                │  │
│  │  Crate: crates/ironclaw_engine/                                       │  │
│  │  Replaces ~10 abstractions with 5 primitives:                         │  │
│  │    Thread (unit of work) | Step (one LLM call + actions)              │  │
│  │    Capability (unit of effect) | MemoryDoc (durable knowledge)        │  │
│  │    Project (context scope)                                            │  │
│  │                                                                       │  │
│  │  Two execution tiers:                                                 │  │
│  │    Tier 0: Structured tool calls (standard function calling)          │  │
│  │    Tier 1: Embedded Python via Monty (CodeAct/RLM pattern)            │  │
│  │                                                                       │  │
│  │  Tier 1 CodeAct model:                                                │  │
│  │    - Context injected as Python variables (not attention input)        │  │
│  │    - llm_query(prompt, ctx) - recursive sub-agent LLM call            │  │
│  │    - rlm_query(prompt) - sub-agent with tool access                   │  │
│  │    - FINAL(answer) - signals completion                               │  │
│  │    - Unknown function calls -> lease check -> policy -> execute        │  │
│  │    - Resource limits: 30s timeout, 64MB memory, 1M allocations        │  │
│  │                                                                       │  │
│  │  Capability leases (replacing static tool permissions):               │  │
│  │    CapabilityLease { thread_id, actions, expires_at, max_uses }       │  │
│  │    PolicyEngine: Deny > RequireApproval > Allow (deterministic)       │  │
│  │                                                                       │  │
│  │  Learning missions (auto-fired on thread completion):                 │  │
│  │    1. error-diagnosis - on threads with trace issues                  │  │
│  │    2. skill-extraction - on success with 5+ steps + 3+ actions        │  │
│  │    3. conversation-insights - every 5 completed threads               │  │
│  └───────────────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────────────────┘
```

### What the LLM sees each turn

```
System Prompt
├── Identity files (AGENTS.md, SOUL.md, USER.md, IDENTITY.md, MEMORY.md)
├── Channel hints (formatting for Discord, Telegram, etc.)
├── Model/platform info
├── Active skill context (<skill> blocks)
├── Active extension listings
└── Tool definitions (JSON Schema per tool)
```

Tool list filtering pipeline:

```
ToolRegistry::tool_definitions()          <- all registered (builtin + WASM + MCP)
  -> attenuate_tools(skills)              <- strip dangerous if INSTALLED skill active
  -> filter by ToolDomain                 <- Orchestrator vs Container
  -> filter disabled (PermissionState)    <- user can disable specific tools
  -> exclude AUTONOMOUS_TOOL_DENYLIST     <- for background jobs/routines
```

### Tool trait (`src/tools/tool.rs`)

Every tool implements:

| Method | Purpose |
|--------|---------|
| `name()` | Tool identifier |
| `description()` | Shown to LLM |
| `parameters_schema()` | JSON Schema for arguments |
| `execute(params, ctx)` | Run the tool, return `ToolOutput` |
| `requires_sanitization()` | Whether output passes through safety layer (default: true) |
| `risk_level_for(params)` | `Low` / `Medium` / `High` |
| `requires_approval(params)` | `Never` / `UnlessAutoApproved` / `Always` |
| `execution_timeout()` | Default 60s |
| `domain()` | `Orchestrator` (main process) or `Container` (Docker worker) |

### Tool permission model

`PermissionState` per tool: `AlwaysAllow` / `AskEachTime` / `Disabled`. Per-user overrides stored in DB settings. `Disabled` tools are not executed. `AskEachTime` triggers the interactive approval flow.

Autonomous denylist (blocked from background jobs/routines): `routine_create/update/delete/fire`, `event_emit`, `create_job`, `job_prompt`, `restart`, `tool_install/auth/activate/remove/upgrade`, `skill_install/remove`, `secret_list/delete`.

---

## 4. Agent Loop

### Complete message flow

```
External Source (Telegram msg, browser chat, CLI input, webhook)
     │
     ▼
Channel.start() -> Stream<IncomingMessage>
IncomingMessage { user_id, channel, content, thread_id, attachments... }
     │
     ▼
ChannelManager (merges all channel streams via StreamExt)
     │
     ▼
┌─────────────────────────────────────────────────────────────────────┐
│  Agent::run() — main event loop (src/agent/agent_loop.rs)          │
│                                                                     │
│  tokio::select! {                                                   │
│    ctrl_c   -> shutdown                                             │
│    msg      -> handle_message(&msg)                                 │
│  }                                                                  │
│                                                                     │
│  Background tasks:                                                  │
│    - SelfRepair (stuck jobs + broken tools, periodic)               │
│    - Session pruning (every 10 min, warn at 1000 sessions)          │
│    - Heartbeat (reads HEARTBEAT.md, proactive notifications)        │
│    - RoutineEngine + cron ticker (scheduled/reactive tasks)         │
└─────────────────────────────────────────────────────────────────────┘
     │
     ▼
┌─────────────────────────────────────────────────────────────────────┐
│  handle_message() dispatch pipeline                                 │
│                                                                     │
│  1. Internal message?        -> forward directly (skip processing)  │
│  2. Set tool context         -> message tool channel/target         │
│  3. SubmissionParser::parse  -> typed Submission variant             │
│  4. BeforeInbound hook       -> can modify or reject content        │
│  5. Engine V2 routing?       -> bridge::handle_with_engine()        │
│  6. Thread hydration         -> load from DB, verify ownership      │
│  7. Auth mode interception   -> pending_auth -> credential store    │
│  8. Event trigger check      -> fire matching routine triggers      │
│  9. Submission dispatch:                                            │
│                                                                     │
│     ┌──────────────┬──────────────────────────────────────────┐     │
│     │ Submission   │ Handler                                  │     │
│     ├──────────────┼──────────────────────────────────────────┤     │
│     │ UserInput    │ process_user_input() -> agentic loop     │     │
│     │ Approval     │ process_approval() -> resume paused loop │     │
│     │ Interrupt    │ process_interrupt() -> stop current turn  │     │
│     │ Undo/Redo    │ UndoManager operations                   │     │
│     │ Compact      │ ContextCompactor                         │     │
│     │ SystemCmd    │ handle_system_command() (no session lock) │     │
│     │ JobStatus    │ scheduler.job_status()                    │     │
│     │ Quit         │ Ok(None) -> breaks main loop             │     │
│     └──────────────┴──────────────────────────────────────────┘     │
└─────────────────────────────────────────────────────────────────────┘
     │
     ▼  (for UserInput submissions)
┌─────────────────────────────────────────────────────────────────────┐
│  process_user_input() (src/agent/thread_ops.rs)                     │
│                                                                     │
│  1. Check thread state (Processing -> queue; AwaitingApproval ->    │
│     pending; Completed -> error)                                    │
│  2. Safety: validate_input() + check_policy() +                     │
│     scan_inbound_for_secrets()                                      │
│  3. Router: route_command() for explicit /commands                   │
│  4. Auto-compact if ContextMonitor suggests (>80% context used)     │
│  5. UndoManager.checkpoint() (snapshot before turn)                 │
│  6. augment_with_attachments() (images, transcripts)                │
│  7. thread.start_turn(content) -> turn_messages                     │
│  8. Persist user message to DB immediately                          │
│  9. channels.send_status(Thinking)                                  │
│  10. ────────────> run_agentic_loop() <──────────────               │
│  11. Post-loop: record response, update thread, persist, TurnCost   │
│  12. Drain loop: if messages queued during Processing, process next  │
└─────────────────────────────────────────────────────────────────────┘
     │
     ▼
┌─────────────────────────────────────────────────────────────────────┐
│  run_agentic_loop() — shared engine (src/agent/agentic_loop.rs)     │
│  max 50 iterations                                                  │
│                                                                     │
│  Three delegates share this loop:                                   │
│    ChatDelegate     - interactive turns (src/agent/dispatcher.rs)   │
│    JobDelegate      - background jobs (src/worker/job.rs)           │
│    ContainerDelegate - Docker workers (src/worker/container.rs)     │
│                                                                     │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │  for iteration in 1..=50:                                    │  │
│  │                                                               │  │
│  │  1. check_signals()                                           │  │
│  │     ChatDelegate: thread.state == Interrupted? -> Stop        │  │
│  │     JobDelegate:  rx.try_recv() Stop/UserMessage              │  │
│  │     ContainerDel: similar signal check                        │  │
│  │                                                               │  │
│  │  2. before_llm_call()                                         │  │
│  │     - Refresh tool definitions (new tools become visible)     │  │
│  │     - Apply skill-based tool attenuation                      │  │
│  │     - Load per-user tool permissions from DB                  │  │
│  │     - Inject iteration-limit nudge at max_tool_iter - 1       │  │
│  │     - Force force_text at max_tool_iterations                 │  │
│  │     - Send Thinking(step N) status                            │  │
│  │                                                               │  │
│  │  3. call_llm()                                                │  │
│  │     CostGuard::check_allowed() (daily budget + hourly rate)   │  │
│  │     Load per-user model override                              │  │
│  │     Reasoning::respond_with_tools(reason_ctx)                 │  │
│  │       -> build system prompt + skill context + tools          │  │
│  │       -> LlmProvider::complete_with_tools(request)            │  │
│  │       -> Provider chain: Retry -> SmartRoute -> Failover      │  │
│  │          -> CircuitBreaker -> Cache -> (actual API call)      │  │
│  │       -> clean_response() (strip thinking/reasoning tags)     │  │
│  │     On ContextLengthExceeded: compact + retry (once)          │  │
│  │     Record cost via CostGuard + persist to DB                 │  │
│  │                                                               │  │
│  │              ┌─────────┴─────────┐                            │  │
│  │              │                   │                            │  │
│  │         ToolCalls              Text                           │  │
│  │              │                   │                            │  │
│  │  4a. Text:                      │                            │  │
│  │     Tool intent nudge check     │                            │  │
│  │     ("let me search..." with    │                            │  │
│  │      no tool call -> nudge,     │                            │  │
│  │      up to 2x)                  │                            │  │
│  │     Else -> Return Response     │                            │  │
│  │                                 │                            │  │
│  │  4b. ToolCalls:                                               │  │
│  │     finish_reason == Length?                                   │  │
│  │       -> discard truncated, inject recovery msg               │  │
│  │       -> after 3 truncations: force_text = true               │  │
│  │                                                               │  │
│  │     Phase 1 - PREFLIGHT (sequential):                         │  │
│  │       BeforeToolCall hook -> modify or reject                 │  │
│  │       Approval check per tool:                                │  │
│  │         Never -> Runnable                                     │  │
│  │         UnlessAutoApproved + approved -> Runnable              │  │
│  │         Always / not approved -> NeedsApproval                │  │
│  │         Disabled -> Rejected                                  │  │
│  │       First NeedsApproval -> LoopOutcome::NeedApproval        │  │
│  │                                                               │  │
│  │     Phase 2 - PARALLEL EXEC (JoinSet):                        │  │
│  │       For each Runnable tool:                                 │  │
│  │         execute_tool_with_safety()                            │  │
│  │           -> prepare_tool_params()                            │  │
│  │           -> safety.validate_tool_params()                    │  │
│  │           -> redact_params() for logging                      │  │
│  │           -> timeout(tool.execute(params, job_ctx))           │  │
│  │                                                               │  │
│  │     Phase 3 - POST-FLIGHT:                                    │  │
│  │       process_tool_result()                                   │  │
│  │         -> safety.sanitize_tool_output()                      │  │
│  │         -> safety.wrap_for_llm() (<tool_output> XML)          │  │
│  │         -> ChatMessage::tool_result()                         │  │
│  │       Emit ToolCompleted + ToolResult status updates          │  │
│  │       Record to DB via store.record_tool_call()               │  │
│  │       -> Continue loop (LLM sees results next iteration)      │  │
│  │                                                               │  │
│  │  5. after_iteration() (optional bookkeeping)                  │  │
│  │                                                               │  │
│  │  ---- repeat ----                                             │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                                                                     │
│  LoopOutcome:                                                       │
│    Response(String)              <- completed with text             │
│    Stopped                       <- cancelled by signal             │
│    MaxIterations                 <- hit 50 iterations               │
│    Failure(String)               <- hard failure                    │
│    NeedApproval(PendingApproval) <- paused for user OK             │
└─────────────────────────────────────────────────────────────────────┘
     │
     ▼
BeforeOutbound hook -> channels.respond(OutgoingResponse)
or SSE broadcast_for_user() for web gateway
```

### Three delegate comparison

| | ChatDelegate | JobDelegate | ContainerDelegate |
|---|---|---|---|
| **File** | `src/agent/dispatcher.rs` | `src/worker/job.rs` | `src/worker/container.rs` |
| **Trigger** | User message on channel | `create_job` tool / `/job` | Orchestrator spawns Docker |
| **Session lock** | Yes (holds thread mutex) | No (independent) | No (runs in container) |
| **Tool approval** | Interactive (NeedApproval) | Pre-approved (ApprovalContext) | No gate (restricted registry) |
| **Tool execution** | Parallel (JoinSet) | Sequential | Sequential, simple errors |
| **LLM calls** | Direct via provider chain | Direct via provider chain | Proxied via orchestrator HTTP |
| **Skill injection** | Yes (per-turn selection) | No | No |
| **Planning** | No | Optional (`use_planning`) | No |
| **Timeout** | None (user-driven) | Configurable per job | 600s default |
| **Recovery** | Tool intent nudge | AutonomousRecoveryState | AutonomousRecoveryState |
| **Completion** | Text response returned | Job state -> Completed | "The job is complete" phrase |

### LLM provider chain

```
User config (LLM_BACKEND env var)
     │
     ▼
Raw Provider (one of):
  NearAI | OpenAI | Anthropic | GitHub Copilot | Ollama |
  OpenAI-Compatible | Tinfoil | Bedrock | OpenAI Codex
     │
     ▼
RetryProvider         (exponential backoff, honors Retry-After)
     │
     ▼
SmartRoutingProvider  (optional: 13-dim complexity scorer -> cheap vs primary)
     │
     ▼
FailoverProvider      (optional: fallback model with per-provider cooldown)
     │
     ▼
CircuitBreakerProvider (optional: Closed -> Open -> HalfOpen state machine)
     │
     ▼
CachedProvider        (optional: SHA-256 keyed, LRU + TTL eviction)
     │
     ▼
RecordingLlm          (optional: trace capture for E2E replay)
```

### Context management

```
Context pressure detection (ContextMonitor):
  Token estimation: word_count x 1.3 + 4 per message
  Default limit: 100,000 tokens

  80-85% -> MoveToWorkspace (archive to daily/YYYY-MM-DD.md, keep 10 turns)
  85-95% -> Summarize (LLM summary -> workspace daily log, keep 5 turns)
  >95%   -> Truncate (drop oldest turns, keep 3 turns, no LLM call)

  Inline recovery: ContextLengthExceeded error -> compact_messages_for_retry()
  Manual trigger: /compact command
```

---

## Key files reference

| Area | File | Role |
|------|------|------|
| Entry | `src/main.rs` | CLI args, PID lock, channel wiring, main loop |
| Startup | `src/app.rs` | `AppBuilder` 5-phase init |
| Agent | `src/agent/agent_loop.rs` | `Agent` struct, `run()`, `handle_message()` |
| Agentic loop | `src/agent/agentic_loop.rs` | `run_agentic_loop()`, `LoopDelegate` trait |
| Chat delegate | `src/agent/dispatcher.rs` | `ChatDelegate`, skill injection, tool approval |
| User input | `src/agent/thread_ops.rs` | `process_user_input()`, safety, thread hydration |
| Parsing | `src/agent/submission.rs` | `SubmissionParser`, `Submission` enum |
| Scheduler | `src/agent/scheduler.rs` | `Scheduler`, `dispatch_job()`, `WorkerMessage` |
| Job worker | `src/worker/job.rs` | `Worker`, `JobDelegate` |
| Container | `src/worker/container.rs` | `WorkerRuntime`, `ContainerDelegate` |
| LLM | `src/llm/reasoning.rs` | `Reasoning`, system prompt, `respond_with_tools()` |
| LLM providers | `src/llm/mod.rs` | `create_llm_provider()`, `build_provider_chain()` |
| Tool exec | `src/tools/execute.rs` | `execute_tool_with_safety()`, `process_tool_result()` |
| Tool registry | `src/tools/registry.rs` | `ToolRegistry`, registration methods |
| WASM runtime | `src/tools/wasm/runtime.rs` | `WasmToolRuntime`, compilation, resource limits |
| MCP client | `src/tools/mcp/client.rs` | `McpClient`, JSON-RPC, tool wrapping |
| Skills | `crates/ironclaw_skills/` | `SkillRegistry`, selector, gating, parser |
| Safety | `crates/ironclaw_safety/` | `SafetyLayer`, injection defense, leak detection |
| Engine v2 | `crates/ironclaw_engine/` | Thread/Step/Capability/CodeAct model |
| Tenant | `src/tenant.rs` | `TenantScope`, `SystemScope` |
| Ownership | `src/ownership/mod.rs` | `OwnerId`, `Identity`, `OwnershipCache` |
| Channels | `src/channels/channel.rs` | `Channel` trait, `IncomingMessage`, `StatusUpdate` |
| Web gateway | `src/channels/web/` | Browser UI, auth, SSE, admin API |
| Pairing | `src/pairing/` | DM admission gate for WASM channels |
| Compaction | `src/agent/compaction.rs` | `ContextCompactor`, three strategies |
| Hooks | `src/hooks/hook.rs` | `HookPoint`, `HookEvent`, lifecycle hooks |
