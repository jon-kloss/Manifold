# Contributing to MANIFOLD

Thanks for wanting to help — contributions are welcome, from bug reports to
features. This page is the short version of how to land a change smoothly.

## Reporting bugs & requesting features

Open a [GitHub issue](https://github.com/jon-kloss/Conveyancer/issues) using
the matching template. For bugs, the most useful things you can include are:
what you did, what you expected, what happened instead, and whether it was on
the [web version](https://manifold-app.up.railway.app/) or the desktop exe
(and which release version). A screenshot of the map/graph state helps a lot.

## Development setup

The README's **For developers** section covers everything: the recommended
headless flow is `cargo run -p app --no-default-features --bin dev-bridge` in
one terminal and `cd renderer && pnpm dev` in another — no display or GTK
needed. Read `docs/04-sdd.md` (the authoritative design) before larger
changes, and skim `DECISIONS.md` for the judgment calls already made.

Two invariants are load-bearing everywhere; PRs that break them won't land:

- **Rust owns canonical state.** The renderer is a projection patched by
  events — never a second source of truth.
- **Solves never move geometry.** Numbers change; cards don't. Every mutation
  is one undoable step.

## Before you open a PR

CI enforces all of this, so save yourself a round trip:

```sh
cargo fmt --all --check
cargo clippy --workspace --exclude app -- -D warnings
cargo clippy -p app --no-default-features -- -D warnings
cargo test --workspace --exclude app && cargo test -p app --no-default-features
cd renderer && pnpm typecheck && pnpm test
cd renderer && pnpm exec playwright test     # full e2e against the real core
```

Extra gates to know about:

- **Committed WASM**: touching `crates/solver` requires
  `scripts/regen-wasm.sh`; touching the web session closure (`crates/web`,
  `crates/app`, `crates/planner-core`, `crates/solver`, `crates/gamedata`,
  `crates/persist`) requires `scripts/regen-web-wasm.sh`. CI fails on
  source-hash drift.
- **Design tokens**: colors/spacing live in `crates/app/src/tokens.rs` only —
  regen with `cargo run -p app --no-default-features --bin gen-tokens`. No
  hex values outside the token system.
- **Tests are the contract**: a behavior change needs its test changed in the
  same PR, with the reasoning in the commit message. New behavior needs a new
  test — Rust-level where possible, e2e where the UI is the point.

## PR conventions

- Keep PRs focused; explain **why** in the description, not just what.
- If you made a judgment call beyond the docs, add a one-line entry to
  `DECISIONS.md`. Deliberate deferrals go to `BACKLOG.md`.
- CI must be fully green; the maintainer reviews and squash-merges.

## Licensing of contributions

The project is dual-licensed **MIT OR Apache-2.0**. Unless you explicitly
state otherwise, any contribution you intentionally submit for inclusion is
licensed the same way, without additional terms (Apache-2.0 §5). Game-derived
assets are third-party content — see `NOTICE.md` — and are not covered by the
project license; don't add game assets without a notice entry.
