# Engine v2 Architecture

This document describes the IronClaw Engine v2 architecture for new contributors. It covers the execution model, the Python orchestrator, the bridge layer, and how everything fits together.

## Overview

IronClaw Engine v2 replaces ~10 fragmented abstractions (Session, Job, Routine, Channel, Tool, Skill, Hook, Observer, Extension, LoopDelegate) with a unified model built on 5 primitives. The engine lives in `crates/ironclaw_engine/` as a standalone crate with no dependency on the main `ironclaw` crate.

The key architectural innovation: **the execution loop is Python code running inside the Monty interpreter, not Rust**. Rust provides the infrastructure (LLM calls, tool execution, safety, persistence). Python provides the orchestration (tool dispatch, output formatting, state management). This makes the glue layer self-modifiable at runtime by the self-improvement Mission.

## Five Primitives

| Primitive | Purpose | Replaces |
|-----------|---------|----------|
| **Thread** | Unit of work with lifecycle, parent-child tree, capability leases | Session + Job + Routine + Sub-agent |
| **Step** | Unit of execution (one LLM call + its action executions) | Agentic loop iteration + tool calls |
| **Capability** | Unit of effect (actions + knowledge + policies) | Tool + Skill + Hook + Extension |
| **MemoryDoc** | Unit of durable knowledge (summaries, lessons, playbooks) | Workspace memory blobs |
| **Project** | Unit of context (scopes memory, threads, missions) | Flat workspace namespace |

## Execution Model

### The Two-Layer Architecture

```
Rust Layer (stable kernel — rarely changes)
  ├── LlmBackend trait     → make LLM API calls
  ├── EffectExecutor trait  → run tools with safety/policy/hooks
  ├── Store trait           → persist threads, steps, events, docs
  ├── LeaseManager          → grant/check/consume/revoke capability leases
  ├── PolicyEngine          → deterministic allow/deny/require-approval
  ├── ThreadManager         → spawn, stop, inject messages, join threads
  ├── Monty VM              → embedded Python interpreter
  └── Safety layer          → sanitization, leak detection, policy enforcement

Python Layer (self-modifiable orchestrator — where bugs get fixed)
  ├── The step loop         → call LLM → handle response → repeat
  ├── Tool dispatch         → name resolution, alias mapping
  ├── Output formatting     → truncation, context assembly
  ├── State management      → persisted_state dict across code steps
  ├── FINAL() extraction    → parse termination signals from text
  ├── Tool intent nudging   → detect when LLM describes instead of acts
  └── Doc injection         → format memory docs for context
```

### How It Works

1. **Bootstrap** (`ExecutionLoop::run()` in `loop_engine.rs`, ~80 lines):
   - Transition thread to Running state
   - Inject CodeAct system prompt (with runtime prompt overlay if available)
   - Load versioned Python orchestrator from Store (or compiled-in default)
   - Execute orchestrator via Monty VM
   - Map return value to `ThreadOutcome`
   - Persist final state

2. **Orchestrator** (`orchestrator/default.py`, ~230 lines):
   - Calls host functions to interact with Rust infrastructure
   - Runs the step loop: check signals → check budget → call LLM → handle response
   - For text responses: extract FINAL(), check nudge, or complete
   - For code responses: run user code in nested Monty VM, format output
   - For action calls: execute each action, handle approval flow
   - Returns outcome dict: `{outcome, response, error, ...}`

3. **Host functions** (Rust, called via Monty's suspension mechanism):
   - `__llm_complete__` → call `LlmBackend::complete()`
   - `__execute_code_step__` → run user CodeAct code in a nested Monty VM
   - `__execute_action__` → execute a tool with lease + policy + safety
   - `__check_signals__` → poll for stop/inject signals
   - `__emit_event__` → broadcast ThreadEvent + record in thread
   - `__add_message__` → append message to thread history
   - `__save_checkpoint__` → persist state to thread metadata
   - `__transition_to__` → validated thread state transition
   - `__retrieve_docs__` → query memory docs from Store
   - `__check_budget__` → remaining tokens/time/USD
   - `__get_actions__` → available tool definitions from leases

### Nested Execution (CodeAct)

When the LLM responds with Python code, the orchestrator calls `__execute_code_step__(code, state)`. This suspends the orchestrator VM and creates a **second Monty VM** for the user's code:

```
Orchestrator VM (Monty #1)
  → calls __execute_code_step__(code, state)
  → suspends
      → Rust creates Monty #2 (user code VM)
      → User code calls web_search() → suspends → Rust executes tool → resumes
      → User code calls FINAL("answer") → terminates
      → Rust collects results
  → Orchestrator VM resumes with results dict
  → Orchestrator formats output, decides next step
```

This is the same mechanism as `rlm_query()` (recursive sub-agent). Each VM owns its own heap — no shared state, no locks.

### Thread State Machine

```
Created → Running → Waiting → Running (resume)
                  → Suspended → Running (resume)
                  → Completed → Reflecting → Done
                  → Failed
```

Terminal states: `Done`, `Failed`. Validated by `ThreadState::can_transition_to()`.

## Bridge Layer (`src/bridge/`)

The bridge connects the engine to existing IronClaw infrastructure:

| Adapter | Wraps | Purpose |
|---------|-------|---------|
| `LlmBridgeAdapter` | `LlmProvider` | Converts `ThreadMessage` ↔ `ChatMessage`, depth-based model routing, code block detection |
| `EffectBridgeAdapter` | `ToolRegistry` + `SafetyLayer` | Tool execution with all v1 security controls, name normalization (underscore ↔ hyphen), rate limiting |
| `HybridStore` | `Workspace` | In-memory for ephemeral data, workspace files for MemoryDocs |
| `EngineRouter` | `Agent` | Routes messages through engine when `ENGINE_V2=true`, manages SSE events |

### Enabling Engine v2

Set `ENGINE_V2=true` environment variable. The router in `src/bridge/router.rs` intercepts messages and routes them through the engine instead of the v1 agent loop.

For trace debugging: `ENGINE_V2_TRACE=1` writes full JSON traces to `engine_trace_*.json`.

## Memory and Reflection

### MemoryDoc Types

| Type | Purpose | Produced By |
|------|---------|-------------|
| `Summary` | What a thread accomplished | Reflection (always) |
| `Lesson` | Durable learning from experience | Reflection (on errors) |
| `Playbook` | Reusable multi-step procedure | Reflection (on success with 2+ tools) |
| `Issue` | Detected problem for follow-up | Reflection (on failure) |
| `Spec` | Missing capability request | Reflection (on "not found" errors) |
| `Note` | Working memory / scratch | Self-improvement, orchestrator code |

### Reflection Pipeline

After a thread completes with `enable_reflection: true`:

1. **Trace analysis** (non-LLM, always runs) — detects 8 issue categories
2. **LLM reflection** — spawns a Reflection-type CodeAct thread with read-only tools
3. **Doc production** — creates Summary, Lesson, Issue, Spec, Playbook docs
4. **Persistence** — saves docs to Store (HybridStore → workspace files)
5. **Event firing** — if issues detected, fires OnSystemEvent missions (self-improvement)

### Context Injection

On each LLM call, `build_step_context()` retrieves up to 5 relevant MemoryDocs from the project and appends them to the system prompt as "## Prior Knowledge". This gives the LLM access to lessons, playbooks, and known issues from prior threads.

## Missions

Missions are long-running goals that spawn threads over time. They replace v1 Routines.

```
Mission
  ├── goal: "Increase test coverage to 80%"
  ├── cadence: Cron("0 9 * * *") | OnSystemEvent | Manual | Webhook
  ├── current_focus: "Write tests for auth module"  (evolves)
  ├── approach_history: ["Analyzed codebase", "Added 15 tests for db"]
  ├── thread_history: [thread_1, thread_2, ...]
  └── max_threads_per_day: 10
```

### How Missions Fire

- **Cron**: Background ticker checks every 60s, fires missions with past `next_fire_at`
- **OnSystemEvent**: Event listener subscribes to ThreadManager events, fires matching missions when threads complete with issues
- **Manual**: `mission_fire(id)` from CodeAct or API
- **Webhook**: Bridge routes incoming webhooks to matching missions

### Meta-Prompt Generation

When a mission fires, `build_meta_prompt()` assembles:
- Mission goal + success criteria
- Current focus (what to work on next)
- Approach history (what was tried and what happened)
- Project knowledge (relevant MemoryDocs)
- Trigger payload (event data, trace issues)

The thread runs with this context and returns: what it accomplished, what to focus on next, whether the goal is achieved. `process_mission_outcome()` extracts these and updates the mission.

## Capability System

### Leases

Threads don't have static permissions. They receive **leases** — scoped, time-limited, use-limited grants:

```rust
CapabilityLease {
    thread_id,
    capability_name,
    granted_actions: ["web_search", "read_file", ...],
    expires_at: Option<DateTime>,
    max_uses: Option<u32>,
    revoked: bool,
}
```

### Policy Engine

The PolicyEngine evaluates actions against leases deterministically:

1. Check global denied effects (e.g., deny all Financial)
2. Check capability-level policies (per-action rules)
3. Check action's `requires_approval` flag
4. Check effect types against lease grant

Decision priority: **Deny > RequireApproval > Allow**

### Effect Types

Every action declares its side effects:
```
ReadLocal, ReadExternal, WriteLocal, WriteExternal,
CredentialedNetwork, Compute, Financial
```

## Integration Scaling Strategy

### The Problem: Tool List Bloat

A naive approach to adding third-party integrations (Slack, GitHub, Stripe, etc.) is to register each API action as a separate tool — `slack_post_message`, `slack_list_channels`, `github_create_issue`, etc. This fails for LLM-based agents:

- Each tool definition costs ~80-120 tokens in the tool list, sent on **every request**
- 200 actions = ~20,000 tokens always-on context cost
- LLM tool selection accuracy **degrades significantly** beyond ~20-30 tools
- The LLM still has to construct correct parameters — deterministic execution doesn't help if the LLM picks the wrong tool or hallucinates params

This was confirmed by studying [Pica](https://github.com/withoneai/pica) (formerly IntegrationOS), which supports 200+ platforms via data-driven definitions in MongoDB. Pica's approach works for programmatic API access, but registering all those actions as LLM tools would degrade agent performance.

### The Solution: Capabilities as Knowledge-Bearing Definitions

In engine v2, Capabilities replace both Tools and Skills. A Capability bundles **actions** (what it can do) with **knowledge** (how to do it). For API integrations, this means:

1. The `http` action is always available (one tool in the LLM's action list)
2. Each integration is a Capability with knowledge text that teaches the LLM how to call that platform's API
3. Capabilities are loaded on-demand based on thread context, not registered globally
4. The LLM reads the knowledge, constructs the correct `http` call

```
User: "post hello to #general on slack"
       ↓
Capability activation: "slack-api" loaded into thread context
       ↓
LLM reads knowledge: learns endpoints, auth pattern, body format
       ↓
LLM calls `http` action:
  POST https://slack.com/api/chat.postMessage
  headers: {"Authorization": "Bearer {SLACK_BOT_TOKEN}"}
  body: {"channel": "C01234", "text": "hello"}
       ↓
EffectExecutor: policy check → credential injection → SSRF protection → leak detection → response
```

### Token Cost Comparison

| Scenario | Dedicated Tools (200 actions) | Capability + http |
|---|---|---|
| User asks about Slack | ~20,000 (all tools in list) | ~700 (http action + slack knowledge) |
| User asks about nothing | ~20,000 (still there) | ~200 (just http action) |
| Tool selection accuracy | Degrades with count | Always picks `http` — no confusion |
| Adding a new platform | Define N tool schemas + executor | Write knowledge text (markdown) |

### What a Capability Definition Looks Like

```yaml
name: slack-api
description: Slack Web API — post messages, manage channels, search, react
knowledge: |
  Base: `https://slack.com/api`
  Auth header: `Authorization: Bearer {SLACK_BOT_TOKEN}`
  All POST bodies are JSON with `Content-Type: application/json`.

  **Post message**: POST `/chat.postMessage` body `{"channel":"<id>","text":"<msg>"}`
  **List channels**: GET `/conversations.list?types=public_channel&limit=100`
  **Search**: GET `/search.messages?query=<text>`
  **Add reaction**: POST `/reactions.add` body `{"channel":"<id>","timestamp":"<ts>","name":"<emoji>"}`

  All responses: `{"ok": true, ...}` or `{"ok": false, "error": "<code>"}`.
  Paginate with `cursor` param when `response_metadata.next_cursor` is non-empty.
actions: [http]
effects: [CredentialedNetwork, ReadExternal, WriteExternal]
policies:
  requires_secret: SLACK_BOT_TOKEN
```

~350 tokens of knowledge covers 4+ actions. The LLM generalizes the pattern to other Slack endpoints from training data.

### Classification of v1 Built-in Tools

Studied all 37 v1 built-in tools to determine which fit the knowledge-driven pattern:

**Can be knowledge-driven (HTTP API wrappers):**
- `image_gen`, `image_analyze`, `image_edit` — pure HTTP calls to external APIs with auth

**Already a generic action (the execution engine):**
- `http` — the action that knowledge-driven Capabilities delegate to

**Must remain dedicated actions (complex local logic):**
- `shell` — 4-layer command validation, Docker sandbox, environment scrubbing
- `file` (read/write/list/patch) — local filesystem with path traversal prevention
- `memory_*` — hybrid FTS + vector search, prompt injection detection
- `job_*` — Docker container lifecycle, context isolation
- `routine_*` — database-backed CRON scheduling
- `extension_tools`, `skill_tools` — registry and system management
- `secrets_tools` — encrypted store management
- `json`, `time`, `echo` — pure local computation
- `message`, `restart`, `tool_info` — internal agent control

**Takeaway**: Only 3 of 37 existing tools are HTTP wrappers. The value is not converting existing tools — it's enabling hundreds of **new** integrations (Slack, GitHub, Jira, Stripe, Salesforce, etc.) without writing Rust.

### Where Dedicated Actions Still Win

1. **Autonomous/headless threads** — Missions and background threads with no human oversight benefit from deterministic execution for their 1-2 critical integrations. Register those specific actions via leases.
2. **OAuth token acquisition** — The LLM cannot perform redirect-based OAuth flows. A dedicated `oauth_init` action handles the redirect dance and stores tokens in the secrets system. The Capability knowledge then instructs the LLM to call `oauth_init` before using the API.
3. **High-frequency reliability-critical paths** — If a specific integration is called thousands of times and must never fail, a dedicated action avoids LLM reasoning variance.

### Comparison with Pica's Approach

[Pica](https://github.com/withoneai/pica) uses a data-driven model where each API action is a MongoDB document (`ConnectionModelDefinition`) with base URL, path, method, auth method, schemas, and JavaScript transform functions. A generic executor dispatches requests. Key patterns:

- **Handlebars secret injection** — entire definition rendered as template with user's secrets as context
- **Passthrough + Unified dual mode** — raw HTTP proxy or normalized CRUD via CommonModels
- **JS sandbox transforms** — `fromCommonModel`/`toCommonModel` functions for data mapping
- **`knowledge` field** — free-text documentation per action for AI tool discovery

Pica's model is optimized for programmatic API access (SDK calls from code). For LLM agents, the Capability-as-knowledge approach is superior because it avoids tool list bloat while leveraging the LLM's ability to construct HTTP calls from documentation. The two approaches share the insight that **integrations should be data, not code**.

## Key Files

| File | Purpose |
|------|---------|
| `crates/ironclaw_engine/orchestrator/default.py` | The Python execution loop (v0) |
| `crates/ironclaw_engine/src/executor/orchestrator.rs` | Host functions + versioning + loading |
| `crates/ironclaw_engine/src/executor/loop_engine.rs` | Bootstrap (loads + runs orchestrator) |
| `crates/ironclaw_engine/src/executor/scripting.rs` | Monty VM integration, user code execution |
| `crates/ironclaw_engine/src/runtime/manager.rs` | ThreadManager (spawn, stop, join, reflection) |
| `crates/ironclaw_engine/src/runtime/mission.rs` | MissionManager (lifecycle, firing, self-improvement) |
| `crates/ironclaw_engine/src/types/` | All core data structures |
| `crates/ironclaw_engine/src/traits/` | LlmBackend, Store, EffectExecutor |
| `src/bridge/router.rs` | Engine v2 entry point from main crate |
| `src/bridge/effect_adapter.rs` | Tool execution bridge with safety |
| `src/bridge/llm_adapter.rs` | LLM provider bridge |
| `src/bridge/store_adapter.rs` | HybridStore (in-memory + workspace) |

## Testing

```bash
cargo check -p ironclaw_engine                                    # compiles
cargo clippy -p ironclaw_engine --all-targets -- -D warnings     # zero warnings
cargo test -p ironclaw_engine                                     # 189 tests
cargo clippy --all --all-features                                 # full crate
cargo test                                                        # full suite
```

## Design Influences

- **RLM paper** (arXiv:2512.24601) — context as variable, FINAL() termination, recursive sub-calls
- **karpathy/autoresearch** — the self-improvement loop as a program.md, fixed-budget evaluation, git as state machine
- **Official RLM impl** (alexzhang13/rlm) — 30 max iterations, compaction at 85%, budget inheritance
- **fast-rlm** (avbiswas/fast-rlm) — Step 0 orientation, parallel sub-calls, dual model routing
- **Pica/IntegrationOS** (withoneai/pica) — data-driven integration definitions, Handlebars secret injection, knowledge fields for AI tool discovery. Validated the "integrations as data" principle; diverged on execution model (knowledge-driven Capabilities instead of per-action tool registration)

See also: `docs/plans/2026-03-20-engine-v2-architecture.md` for the full 8-phase roadmap.
