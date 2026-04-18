# `pi-ai` design notes

## Goal

`rust/crates/pi-ai` is the Rust replacement for `packages/ai`.

The implementation is a rewrite that preserves observable behavior where practical, not a line-by-line translation.

## Built-in model catalog

The Rust built-in catalog is self-contained and maintained in-tree.

- source file loaded at runtime: `rust/crates/pi-ai/src/models.catalog.json`
- loader and filtering logic: `rust/crates/pi-ai/src/models.rs`
- ownership/consistency tests: `rust/crates/pi-ai/tests/models.rs`
- TypeScript generator reference: `packages/ai/scripts/generate-models.ts`
- sync policy: `migration/packages/model-catalog-sync.md`

Operationally, this means:

- no runtime dependency on `packages/ai/src/models.generated.ts`
- no build step that imports TypeScript-generated model artifacts into the Rust workspace
- no maintenance step that requires running the TypeScript generator from Rust
- Rust catalog edits happen directly in `models.catalog.json`

The TypeScript implementation remains the behavior and compatibility reference, but not the runtime source for Rust model metadata.

## Supported built-in providers

Until more providers are migrated, the Rust built-in catalog is intentionally limited to:

- `anthropic`
- `openai`
- `openai-codex`

## Catalog maintenance workflow

When changing built-in model metadata for migrated providers:

1. inspect the TypeScript compatibility source in `packages/ai/scripts/generate-models.ts` and, when useful, `packages/ai/src/models.generated.ts`
2. apply the equivalent Rust-side metadata update directly in `rust/crates/pi-ai/src/models.catalog.json`
3. run `cd rust && cargo run -p pi-ai --bin pi-ai-catalog -- fmt`
4. run `cd rust && cargo run -p pi-ai --bin pi-ai-catalog -- check`
5. keep provider filtering in `rust/crates/pi-ai/src/models.rs` aligned with the migrated provider set
6. update or extend `rust/crates/pi-ai/tests/models.rs`
7. use the TypeScript package as a compatibility reference, not as an input artifact

For the full manual sync contract between the TypeScript generator and the Rust-owned catalog, see `migration/packages/model-catalog-sync.md`.

## Rust-native maintenance command

`rust/crates/pi-ai/src/bin/pi-ai-catalog.rs` is the maintenance entrypoint for the built-in catalog. It provides:

- semantic validation for the Rust-owned catalog schema
- canonical formatting for `models.catalog.json`
- provider/model inventory summaries for the Rust catalog

That gives the Rust workspace its own maintenance path instead of relying on TypeScript-generated model data.
