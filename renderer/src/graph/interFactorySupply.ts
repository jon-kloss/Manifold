// Shared inter-factory supply wiring, used by both entry points: "send to
// another factory" (from a source OUT port) and "receive from another factory"
// (from a target IN port). Both resolve to the same operation — bind one or
// more of the source's OUT ports to the target's IN ports (by item), creating
// the IN ports that don't exist yet so a target can accumulate several inputs.

import type { Command, Factory, Id, Plan, RouteKind } from "../state/types";

type Dispatch = (cmds: Command[]) => Promise<Id[] | null>;

/** Bind each of `outPortIds` (OUT ports on `source`) to a free IN port on
 *  `target` carrying the same item, creating the IN port when none exists.
 *  Missing IN ports are created in one batch (created ids come back in order),
 *  then every route is committed in a second batch — at most two commits total,
 *  independent of how many outputs cross. Returns the created route ids (empty
 *  on a refusal). */
export async function wireSupply(
  plan: Plan,
  dispatch: Dispatch,
  source: Factory,
  target: Factory,
  outPortIds: Id[],
  kind: RouteKind,
  /** The IN port the flow was launched from (RECEIVE) — bound first for its
   *  item so we wire the port the user actually clicked, not a same-item sibling. */
  preferInPort?: Id,
): Promise<Id[]> {
  // Match each output to a free IN port by item; remember what we consumed so
  // two outputs of the same item can't grab the same port twice.
  const usedIn = new Set<Id>();
  const prefer = preferInPort ? plan.ports[preferInPort] : undefined;
  const specs = outPortIds
    .map((pid) => plan.ports[pid])
    .filter((p): p is NonNullable<typeof p> => !!p)
    .map((p) => {
      const canUse = (q: (typeof plan.ports)[string] | undefined) =>
        !!q && q.direction === "in" && !q.boundRoute && q.item === p.item && !usedIn.has(q.id);
      // Prefer the launch IN port for the first output that matches its item.
      const match =
        prefer && canUse(prefer)
          ? prefer
          : target.ports.map((id) => plan.ports[id]).find(canUse);
      if (match) usedIn.add(match.id);
      return { from: p.id, item: p.item, inPort: match?.id ?? null };
    });
  if (specs.length === 0) return [];

  const existingIn = target.ports.filter((id) => plan.ports[id]?.direction === "in").length;
  const toCreate = specs.filter((s) => !s.inPort);
  const createCmds: Command[] = toCreate.map((s, i) => ({
    type: "add_port",
    factory: target.id,
    direction: "in",
    item: s.item,
    rate: 0,
    rateCeiling: null,
    graphPos: { x: 0, y: 80 + (existingIn + i) * 128 },
  }));

  let createdIds: Id[] = [];
  if (createCmds.length) {
    createdIds = (await dispatch(createCmds)) ?? [];
    if (createdIds.length !== createCmds.length) return []; // a refusal — bail before routing
  }

  let ci = 0;
  const path = [source.position, target.position];
  const routeCmds: Command[] = specs.map((s) => ({
    type: "add_route",
    kind,
    from: s.from,
    to: s.inPort ?? createdIds[ci++],
    path,
  }));
  return (await dispatch(routeCmds)) ?? [];
}
