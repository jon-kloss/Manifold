// Floor plates: a translucent, labeled plate behind each floor's cards makes
// the vertical structure of a factory readable at a glance. Identity comes
// from structure (plates, labels, lift counts) — never from new colors; the
// palette stays token-law clean.

import { useMemo } from "react";
import { ViewportPortal } from "@xyflow/react";
import type { BeltEdge, MachineGroup } from "../state/types";

const PAD = 28;
const LABEL_H = 26;

export interface FloorPlatesProps {
  groups: MachineGroup[];
  edges: BeltEdge[];
  /** measured card geometry by group id */
  geoms: Record<string, { x: number; y: number; w: number; h: number }>;
  activeFloor: "all" | number;
}

export default function FloorPlates({ groups, edges, geoms, activeFloor }: FloorPlatesProps) {
  const plates = useMemo(() => {
    const byFloor = new Map<number, MachineGroup[]>();
    for (const g of groups) byFloor.set(g.floor, [...(byFloor.get(g.floor) ?? []), g]);
    if (byFloor.size < 2) return []; // single-floor factories stay clean

    const floorOf = (end: { kind: string; id: string }) =>
      end.kind === "group" ? groups.find((g) => g.id === end.id)?.floor ?? 0 : 0;

    return [...byFloor.entries()]
      .sort((a, b) => a[0] - b[0])
      .map(([floor, members]) => {
        let minX = Infinity;
        let minY = Infinity;
        let maxX = -Infinity;
        let maxY = -Infinity;
        for (const g of members) {
          const geom = geoms[g.id];
          if (!geom) continue;
          minX = Math.min(minX, geom.x);
          minY = Math.min(minY, geom.y);
          maxX = Math.max(maxX, geom.x + geom.w);
          maxY = Math.max(maxY, geom.y + geom.h);
        }
        if (!isFinite(minX)) return null;
        const liftsUp = edges.filter((e) => floorOf(e.from) === floor && floorOf(e.to) > floor).length;
        const liftsDown = edges.filter((e) => floorOf(e.from) === floor && floorOf(e.to) < floor).length;
        const liftsIn = edges.filter((e) => floorOf(e.to) === floor && floorOf(e.from) !== floor).length;
        return { floor, minX, minY, maxX, maxY, liftsUp, liftsDown, liftsIn, count: members.length };
      })
      .filter((p): p is NonNullable<typeof p> => p !== null);
  }, [groups, edges, geoms]);

  if (plates.length === 0) return null;

  return (
    <ViewportPortal>
      {plates.map((p) => {
        const active = activeFloor === "all" || activeFloor === p.floor;
        const liftBits: string[] = [];
        if (p.liftsUp) liftBits.push(`${p.liftsUp}⤒`);
        if (p.liftsDown) liftBits.push(`${p.liftsDown}⤓`);
        if (p.liftsIn) liftBits.push(`${p.liftsIn} IN`);
        return (
          <div
            key={p.floor}
            className={`floor-plate ${active ? "" : "inactive"}`}
            style={{
              transform: `translate(${p.minX - PAD}px, ${p.minY - PAD - LABEL_H}px)`,
              width: p.maxX - p.minX + PAD * 2,
              height: p.maxY - p.minY + PAD * 2 + LABEL_H,
            }}
            data-testid={`floor-plate-${p.floor}`}
          >
            <div className="floor-plate-label">
              <span className="t-label">FLOOR {p.floor}</span>
              <span className="mono floor-plate-meta">
                {p.count} {p.count === 1 ? "GROUP" : "GROUPS"}
                {liftBits.length > 0 && ` · ${liftBits.join(" ")}`}
              </span>
            </div>
          </div>
        );
      })}
    </ViewportPortal>
  );
}
