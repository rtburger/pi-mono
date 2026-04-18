# Model catalog sync policy

## Purpose

Keep `rust/crates/pi-ai/src/models.catalog.json` aligned with the observable behavior of migrated providers without reintroducing a maintenance-time dependency on the TypeScript model generator.

## Ownership boundaries

- `packages/ai/scripts/generate-models.ts` owns TypeScript model generation for `packages/ai`
- `packages/ai/src/models.generated.ts` is the generated TypeScript catalog artifact
- `rust/crates/pi-ai/src/models.catalog.json` is the Rust runtime and maintenance catalog
- `rust/crates/pi-ai/src/bin/pi-ai-catalog.rs` is the Rust formatter/validator/summary entrypoint

Rust may consult the TypeScript files as compatibility references, but it must not import them, execute them, or regenerate the Rust catalog from them.

## Providers currently covered by manual sync

Only migrated Rust providers participate in this sync policy:

- `anthropic`
- `openai`
- `openai-codex`

If a TypeScript model-catalog change affects only non-migrated providers, no Rust catalog update is required.

## Sync triggers

Update the Rust catalog when one of these changes affects a migrated provider:

- logic changes in `packages/ai/scripts/generate-models.ts`
- generated metadata changes in `packages/ai/src/models.generated.ts`
- TypeScript behavior or tests that change expected model ids, names, APIs, base URLs, headers, reasoning flags, input modes, pricing, context windows, or max token limits

## Manual sync procedure

1. Inspect the TypeScript compatibility source:
   - `packages/ai/scripts/generate-models.ts`
   - `packages/ai/src/models.generated.ts` when useful
2. Limit scope to migrated Rust providers only.
3. Apply the equivalent metadata change directly in `rust/crates/pi-ai/src/models.catalog.json`.
4. Validate and normalize the Rust-owned catalog:
   - `cd rust && cargo run -p pi-ai --bin pi-ai-catalog -- fmt`
   - `cd rust && cargo run -p pi-ai --bin pi-ai-catalog -- check`
   - `cd rust && cargo run -p pi-ai --bin pi-ai-catalog -- summary`
5. Re-run Rust catalog coverage:
   - `cd rust && cargo test -p pi-ai --test models`
   - `cd rust && cargo test -p pi-ai --test catalog_cli`
6. If provider coverage changed, update:
   - `rust/crates/pi-ai/src/models.rs`
   - `rust/crates/pi-ai/tests/models.rs`
   - this sync policy and the migration docs that reference it

## Rust-owned fields to keep aligned

When applying a manual sync, compare only the Rust-owned fields that affect runtime behavior:

- `id`
- `name`
- `api`
- `provider`
- `baseUrl`
- `headers`
- `reasoning`
- `input`
- `cost`
- `contextWindow`
- `maxTokens`

Textual identity with the TypeScript catalog is not the goal. Preserve observable behavior for the migrated Rust providers.

## Guardrails

Do not:

- add a Rust build step that runs `npm run generate-models`
- import `packages/ai/src/models.generated.ts` into Rust code
- point Rust runtime loading at TypeScript-generated artifacts
- expand the Rust built-in catalog just to mirror providers that are still TypeScript-only

## Extending this policy

When a new provider is migrated to Rust:

1. add it to `BUILT_IN_MODEL_PROVIDERS`
2. add its entries to `rust/crates/pi-ai/src/models.catalog.json`
3. extend Rust catalog tests
4. add the provider to the manual-sync provider set documented here
