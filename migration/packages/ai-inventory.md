# `pi-ai` inventory

## Runtime-owned assets

The Rust `pi-ai` crate already owns these built-in catalog assets:

- `rust/crates/pi-ai/src/models.catalog.json`
- `rust/crates/pi-ai/src/models.rs`
- `rust/crates/pi-ai/tests/models.rs`

## Current behavior

- built-in models are loaded from the local Rust JSON catalog
- provider filtering is applied in Rust
- cost calculation uses Rust `Model` metadata
- coding-agent model resolution consumes Rust `built_in_models()` output

## Provider coverage in the built-in catalog

Current built-in provider coverage is intentionally limited to the providers with migrated Rust implementations:

- `anthropic`
- `openai`
- `openai-codex`

## Maintenance tooling

The Rust workspace now also owns the catalog maintenance path:

- formatter/validator CLI: `rust/crates/pi-ai/src/bin/pi-ai-catalog.rs`
- default catalog target: `rust/crates/pi-ai/src/models.catalog.json`
- coverage tests: `rust/crates/pi-ai/tests/models.rs` and `rust/crates/pi-ai/tests/catalog_cli.rs`
- manual sync policy: `migration/packages/model-catalog-sync.md`

## Compatibility references

Rust catalog maintenance may consult these TypeScript files for migrated providers:

- `packages/ai/scripts/generate-models.ts`
- `packages/ai/src/models.generated.ts`

Those files are compatibility references only. Rust does not import them, execute them, or regenerate `rust/crates/pi-ai/src/models.catalog.json` from them.

## Ownership statement

The Rust catalog is now the operational source of model metadata for the Rust runtime and for catalog maintenance.

The TypeScript catalog remains useful as a compatibility reference while migration continues, but it is not a runtime dependency and should not be treated as the source file that Rust loads or regenerates from.
