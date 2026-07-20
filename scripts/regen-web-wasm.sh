#!/usr/bin/env bash
# regen-web-wasm.sh — regenerate renderer/src/wasm/web-pkg from crates/web,
# and stamp the sources that fed the build so CI can gate on drift without a
# compiler in the loop. Mirrors scripts/regen-wasm.sh (the solver-wasm pkg):
# rustc/wasm-pack are unpinned, so a byte-diff gate on the 3.4MB committed wasm
# would flake on every toolchain release with zero real drift; a source-hash
# stamp catches the case that matters — crates/web changed (the dispatch arms)
# but the committed binary was not regenerated, so it silently mismatches.
#
# Usage:
#   scripts/regen-web-wasm.sh          # build the pkg with wasm-pack, then stamp
#   scripts/regen-web-wasm.sh check    # recompute the source hash, compare stamp
set -euo pipefail

cd "$(dirname "$0")/.."

STAMP="renderer/src/wasm/web-pkg/.web-src.sha256"

# Hash every source that feeds the web wasm build: crates/web AND the workspace
# crates it statically links (app → solver + gamedata + planner-core + persist).
# The solve/session/gamedata behavior the web pkg ships lives in those deps, not
# just crates/web/src — hashing only the wrapper let a session.rs/solver/gamedata
# change ship a stale binary with a green `check`. Each crate's Cargo.toml is
# included so dep-edge/feature changes are caught too.
src_hash() {
  {
    for c in web app solver gamedata planner-core persist; do
      find "crates/$c/src" -type f -name '*.rs'
      echo "crates/$c/Cargo.toml"
    done
  } | sort | xargs sha256sum | sha256sum | cut -d' ' -f1
}

case "${1:-build}" in
  check)
    expected="$(src_hash)"
    actual="$(cat "$STAMP" 2>/dev/null || echo '<missing>')"
    if [ "$expected" != "$actual" ]; then
      echo "crates/web changed without regenerating renderer/src/wasm/web-pkg — run pnpm --dir renderer build:wasm" >&2
      echo "  stamp:   $actual" >&2
      echo "  sources: $expected" >&2
      exit 1
    fi
    echo "web wasm pkg in sync with crates/web sources ($expected)"
    ;;
  build)
    (cd crates/web && wasm-pack build --target web --out-dir ../../renderer/src/wasm/web-pkg --out-name web --release)
    src_hash > "$STAMP"
    echo "rebuilt renderer/src/wasm/web-pkg and stamped $(cat "$STAMP")"
    ;;
  *)
    echo "usage: $0 [build|check]" >&2
    exit 2
    ;;
esac
