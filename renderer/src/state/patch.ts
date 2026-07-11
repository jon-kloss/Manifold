// TS twin of planner-core::patch::apply — entity-level ops on the projected plan.

import type { Plan, PatchOp } from "./types";

export function applyPatches(plan: Plan, ops: PatchOp[]): Plan {
  const next: Plan = {
    ...plan,
    factories: { ...plan.factories },
    groups: { ...plan.groups },
    ports: { ...plan.ports },
    edges: { ...plan.edges },
    nodeClaims: { ...plan.nodeClaims },
    routes: { ...plan.routes },
    junctions: { ...plan.junctions },
    proposals: { ...plan.proposals },
    switches: { ...plan.switches },
    styleGuides: { ...plan.styleGuides },
    meta: { ...plan.meta },
  };
  for (const op of ops) {
    const path = op.path.replace(/^\//, "");
    const slash = path.indexOf("/");
    if (slash < 0) continue;
    const collection = path.slice(0, slash);
    const key = path.slice(slash + 1);
    if (collection === "meta") {
      if (op.op !== "remove") (next.meta as unknown as Record<string, unknown>)[key] = op.value;
      continue;
    }
    const coll = (next as unknown as Record<string, Record<string, unknown>>)[collection];
    if (!coll) continue;
    if (op.op === "remove") delete coll[key];
    else coll[key] = op.value;
  }
  return next;
}
