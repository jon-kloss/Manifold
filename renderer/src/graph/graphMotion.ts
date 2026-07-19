// MANIFOLD interaction-motion helpers (handoff §5, motions 7h/7k/7l/7m).
// Pure math the GraphView choreography leans on: what appeared/disappeared
// between two renders, the left→right construction order for a MAKE batch,
// and ghost lifecycle pruning. No DOM, no timers — fully unit-testable.

/** One transient ghost: the dashed bp-400 afterimage of a removed node (7h
 *  undo flash / 7k deconstruct collapse) or a retracting removed edge. `at`
 *  is the wall-clock birth; prune after its grammar's total duration. */
export interface NodeGhost {
  id: string;
  kind: "undo" | "delete";
  x: number;
  y: number;
  w: number;
  h: number;
  at: number;
}

export interface EdgeGhost {
  id: string;
  /** SVG path of the edge as it last rendered — the retract replays it. */
  d: string;
  /** Retract direction (7k: "into the survivor"): false = collapse toward
   *  the path END (survivor is the target), true = toward the START. */
  rev: boolean;
  at: number;
}

/** Ids present in `next` but not `prev`, and vice versa. */
export function diffIds(
  prev: ReadonlySet<string>,
  next: ReadonlySet<string>,
): { added: string[]; removed: string[] } {
  const added: string[] = [];
  const removed: string[] = [];
  for (const id of next) if (!prev.has(id)) added.push(id);
  for (const id of prev) if (!next.has(id)) removed.push(id);
  return { added, removed };
}

/** 7m — MAKE chain build order: machine groups construct in production order,
 *  left → right (graph lanes flow left → right, so x IS production order).
 *  Ties break on y then id so the order is total and deterministic. Returns
 *  id → 0-based construction index. */
export function buildOrder(nodes: ReadonlyArray<{ id: string; x: number; y: number }>): Map<string, number> {
  const sorted = [...nodes].sort((a, b) => a.x - b.x || a.y - b.y || (a.id < b.id ? -1 : 1));
  return new Map(sorted.map((n, i) => [n.id, i]));
}

/** Total on-screen life of a node ghost, by grammar:
 *  7h undo flash = ghost 120ms; 7k deconstruct = collapse 150ms + ghost 120ms. */
export function ghostTtlMs(kind: NodeGhost["kind"]): number {
  return kind === "undo" ? 120 : 270;
}

/** 7k edge retract runs 200ms. */
export const EDGE_RETRACT_MS = 200;

/** Drop ghosts whose animation has fully played out. */
export function pruneGhosts<T extends { at: number }>(
  ghosts: readonly T[],
  now: number,
  ttl: (g: T) => number,
): T[] {
  const live = ghosts.filter((g) => now - g.at < ttl(g));
  return live.length === ghosts.length ? [...ghosts] : live;
}

/** A motion verb is only trusted for the exact plan commit that stamped it
 *  (hash = the response's planHash) AND while fresh. Hydrate, sync-import,
 *  auto-pull and proposal-accept all advance planHash WITHOUT stamping, so
 *  their diffs can never be claimed by a stale verb from an earlier edit. */
export const MOTION_FRESH_MS = 1500;

export function motionKind(
  motion: { kind: "edit" | "undo" | "redo"; at: number; hash: string } | null,
  now: number,
  planHash: string,
): "edit" | "undo" | "redo" | null {
  if (!motion || motion.hash !== planHash) return null;
  return now - motion.at < MOTION_FRESH_MS ? motion.kind : null;
}
