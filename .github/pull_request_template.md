## What & why

<!-- What does this change do, and why? Link the issue if there is one. -->

## Checklist

- [ ] `cargo fmt --all --check`, both clippy invocations, and the full Rust
      test suite pass locally
- [ ] `cd renderer && pnpm typecheck && pnpm test` pass
- [ ] e2e (`pnpm exec playwright test`) passes, or the change can't affect it
- [ ] Committed WASM regenerated if I touched its source closure
      (`scripts/regen-wasm.sh` / `scripts/regen-web-wasm.sh`)
- [ ] Behavior changes come with their tests changed/added in this PR
- [ ] Judgment calls beyond the docs got a one-line `DECISIONS.md` entry
