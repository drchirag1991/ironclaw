# PR Audit: feat(workspace): admin system prompt (#2107)

## Executive Summary

- Adds admin system prompt (`SYSTEM.md` in `__admin__` scope) injected into all users' system prompts in multi-tenant mode
- Properly gated behind multi-tenancy (`admin_prompt_enabled` flag + `workspace_pool.is_some()`)
- Injection scanning works correctly for SYSTEM.md writes
- **Two blocking issues found**: missing reserved user ID validation, missing content size limit
- 430 LOC total (78 modified, 352 new), 62% tests

## Blocking Issues

### B1: No reserved user ID validation (CRITICAL)

**File:** `src/channels/web/handlers/users.rs` (user creation)

User IDs are auto-generated UUIDs, so direct collision is unlikely. However, the system has no validation preventing future code paths from creating a user with ID `__admin__`. If such a user existed, their workspace writes would land in the admin system prompt scope.

**Fix:** Add `ADMIN_SCOPE` to a reserved ID check in system_prompt.rs PUT handler — already safe because only AdminUser can call it. But defensively, also validate in user creation that no user gets `__admin__` as their ID.

**Risk:** Low immediate risk (UUIDs are random), but defense-in-depth is warranted.

### B2: No content size limit on SYSTEM.md (MODERATE)

**File:** `src/channels/web/handlers/system_prompt.rs:51-84`

The PUT handler accepts arbitrary content size (up to 10 MB global body limit). A large SYSTEM.md gets injected into every user's system prompt, consuming token budget and potentially causing context overflow.

**Fix:** Add a 64 KB size cap in the PUT handler.

## Non-Blocking Improvements

- None identified. The implementation is clean and minimal.

## File-by-File Notes

| File | Notes |
|------|-------|
| `document.rs` | Clean. `ADMIN_SCOPE` constant with clear docs. Not in `IDENTITY_PATHS`. |
| `mod.rs` (workspace) | `admin_prompt_enabled` properly threaded through constructors, builder, and `scoped_to_user()`. Prompt assembly reads from DB on each call — no stale cache risk. |
| `server.rs` | `.with_admin_prompt()` called in `build_workspace()` — correct since pool only exists in multi-tenant mode. |
| `system_prompt.rs` | Multi-tenancy gate correct. Injection scanning via `ws.write()`. Needs size limit. |
| `types.rs` | Clean DTOs. |
| `handlers/mod.rs` | Module registration only. |
| `tests/admin_system_prompt.rs` | 6 solid tests covering: basic flow, single-user gate, coexistence with identity, empty content, multi-user, scoped_to_user preservation. |

## Release Risk Level

**Low** — feature is additive, gated behind multi-tenant mode, no schema changes, no migration. Single-user deployments are completely unaffected (zero code path executed).

## Recommended Next Action

**Request changes** — fix B1 (reserved ID validation) and B2 (size limit), then approve.
