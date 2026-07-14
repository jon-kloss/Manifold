#!/usr/bin/env bash
# regen-wasm.sh — regenerate renderer/src/wasm/pkg from crates/solver-wasm,
# and stamp the sources that fed the build so CI can gate on drift without
# a compiler in the loop (rustc/wasm-pack are unpinned; a byte-diff gate
# would flake on every toolchain release with zero real drift).
#
# Usage:
#   scripts/regen-wasm.sh          # build the pkg with wasm-pack, then write the stamp
#   scripts/regen-wasm.sh check    # recompute the source hash and compare to the stamp
set -euo pipefail

cd "$(dirname "$0")/.."

STAMP="renderer/src/wasm/pkg/.solver-src.sha256"

# Hash every source that feeds the wasm build. t1.rs is excluded: lib.rs
# cfg-gates it behind the "lp" feature, which is off for the wasm build —
# T1-only changes cannot affect the pkg and must not trip the gate.
src_hash() {
  {
    find crates/solver/src crates/solver-wasm/src -type f -name '*.rs' ! -name 't1.rs'
    echo crates/solver/Cargo.toml
    echo crates/solver-wasm/Cargo.toml
  } | sort | xargs sha256sum | sha256sum | cut -d' ' -f1
}

case "${1:-build}" in
  check)
    expected="$(src_hash)"
    actual="$(cat "$STAMP" 2>/dev/null || echo '<missing>')"
    if [ "$expected" != "$actual" ]; then
      echo "crates/solver changed without regenerating renderer/src/wasm/pkg — run scripts/regen-wasm.sh" >&2
      echo "  stamp:   $actual" >&2
      echo "  sources: $expected" >&2
      exit 1
    fi
    echo "wasm pkg in sync with solver sources ($expected)"
    ;;
  build)
    (cd crates/solver-wasm && wasm-pack build --target web --out-dir ../../renderer/src/wasm/pkg --release)
    src_hash > "$STAMP"
    echo "rebuilt renderer/src/wasm/pkg and stamped $(cat "$STAMP")"
    ;;
  *)
    echo "usage: $0 [build|check]" >&2
    exit 2
    ;;
esac
