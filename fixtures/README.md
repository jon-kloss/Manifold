# Save Fixtures — Provenance & Purpose

Source: test suite of `@etothepii/satisfactory-file-parser` (github.com/etothepii4/satisfactory-file-parser, `src/test/`) — the parser the SDD selects for import (§8). Verified in evaluation: all three parse cleanly with `throwErrors: false`.

These are **Phase 4 fixtures**. Nothing before the importer needs them.

| File | Format | Build | Size | Why it's here |
|---|---|---|---|---|
| `Dunarr-076.sav` | savVer 52 (1.1) | 463028 | 5.4 MB | The workhorse. Real 63-hour playthrough (saved Jan 2026): 21,408 buildables across 467 types — 805 rail segments, 420 constructors, Mk.1–4 belts, pipes, hypertubes, full power grid. **Lightly modded** — exercises the quarantine path (mod objects must be listed + ignored, never fatal). Clustering target: importer should resolve this into a plausible set of logical factories. |
| `269.sav` | savVer 46 (1.0) | 385279 | 5.8 MB | Clean unmodded 1.0-format base of similar scale. Baseline for cluster/count assertions without mod noise; guards format-version tolerance backward. |
| `Another-1-2.sav` | savVer 58 (1.2) | 481836 | 136 KB | Newest save format, near-empty fresh start (74s playtime). Format-compatibility canary: if the newest header/version parses and produces an almost-empty Built layer, forward tolerance holds. |

Suggested importer assertions: exact buildable totals for Dunarr-076 (21,408), non-zero quarantine count on Dunarr-076 with zero on 269.sav, and an empty-but-valid ImportSnapshot from Another-1-2.sav.
