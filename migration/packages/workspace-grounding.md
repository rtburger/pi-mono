# Workspace grounding

This migration stays inside the existing `pi-mono` repository.

## Rust workspace location

- Rust workspace root: `rust/`
- Rust app entrypoint: `rust/apps/pi`
- Rust crates replacing TypeScript packages live under `rust/crates/`

## Rewrite scope

The Rust rewrite targets only these packages:

- `packages/ai` -> `rust/crates/pi-ai`
- `packages/agent` -> `rust/crates/pi-agent`
- `packages/coding-agent` -> `rust/crates/pi-coding-agent-*` and `rust/apps/pi`
- `packages/tui` -> `rust/crates/pi-tui` and `rust/crates/pi-coding-agent-tui`

The TypeScript packages remain in the monorepo as the behavioral reference during migration.

## Model catalog ownership

Built-in Rust model metadata is owned inside the Rust workspace:

- runtime loader: `rust/crates/pi-ai/src/models.rs`
- catalog data: `rust/crates/pi-ai/src/models.catalog.json`

`pi-ai` does not load `packages/ai/src/models.generated.ts` at runtime. The TypeScript catalog is reference material for compatibility checks only.

## Current migrated built-in providers

The Rust built-in catalog currently includes only the providers that have Rust runtime support:

- `anthropic`
- `openai`
- `openai-codex`

That restriction is enforced in `rust/crates/pi-ai/src/models.rs`, the Rust catalog maintenance CLI, and validated in `rust/crates/pi-ai/tests/models.rs`.

## Catalog maintenance commands

Use the Rust-native catalog tool from the workspace root:

- `cd rust && cargo run -p pi-ai --bin pi-ai-catalog -- check`
- `cd rust && cargo run -p pi-ai --bin pi-ai-catalog -- fmt`
- `cd rust && cargo run -p pi-ai --bin pi-ai-catalog -- summary`

This keeps maintenance inside the Rust workspace and avoids any dependency on TypeScript-generated model artifacts.
