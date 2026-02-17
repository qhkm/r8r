## Summary

Describe what changed and why.

## Scope

- [ ] This PR stays within `r8r` scope (not Chronicle-level platform work).
- [ ] I reviewed `AGENTS.md` and followed its validation + documentation rules.

## Risk Classification (required)

Select one:

- [ ] `low` - internal refactor, no contract or behavior change
- [ ] `medium` - behavior change without persistence/contract break
- [ ] `high` - storage/execution semantics, API contract, or security-sensitive logic

If `high`, include rollback/compatibility notes:

<!-- Required for high-risk changes -->

## Required Validation (required)

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test --all-features`
- [ ] If `ui/` changed, I ran the relevant frontend checks/tests.

## Touched-Module Enforcement Matrix (required)

Check all rows that apply and confirm tests/docs were updated.

### `src/workflow/`
- [ ] Added parser/validation tests for schema/constraint changes.
- [ ] Updated docs/examples (`docs/NODE_TYPES.md` and/or `README.md`) when syntax changed.

### `src/nodes/`
- [ ] Added/updated happy-path and failure-path tests.
- [ ] Covered retry/error-handling behavior.
- [ ] Updated `docs/NODE_TYPES.md`.

### `src/triggers/`
- [ ] Added routing/filter edge-case tests.
- [ ] Verified restart/failure behavior.
- [ ] Updated `docs/API.md` for contract changes.

### `src/engine/`
- [ ] Added execution ordering/state transition regression tests.
- [ ] Updated `docs/ARCHITECTURE.md` for behavior model changes.

### `src/storage/`
- [ ] Added compatibility/migration tests for schema/state changes.
- [ ] Added migration notes and updated relevant docs.

### `src/api/`
- [ ] Added endpoint contract/status/error tests.
- [ ] Updated `docs/API.md`.

### `src/mcp/`
- [ ] Added tool input/output contract tests.
- [ ] Updated MCP usage docs in `README.md` or `docs/`.

### `ui/`
- [ ] Added/updated component/integration tests for changed flows.
- [ ] Updated user-facing docs/screenshots when behavior changed materially.

## Documentation and Contracts (required)

- [ ] User-visible behavior changes are documented in `README.md` and/or `docs/`.
- [ ] API contract changes are reflected in `docs/API.md`.
- [ ] Internal planning/strategy notes were kept in `.internal/` (if any).

## Testing Notes

List targeted tests you added/updated and any important coverage limits.

## Untested Risks / Follow-ups

List any known gaps, deferred work, or operational risks for reviewers.
