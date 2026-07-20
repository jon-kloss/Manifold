// Floor plates: a translucent, labeled plate behind each floor's cards makes
// the vertical structure of a factory readable at a glance. Identity comes
// from structure (plates, labels, lift counts) — never from new colors; the
// palette stays token-law clean.

import { useMemo } from "react";
import { ViewportPortal } from "@xyflow/react";
import type { BeltEdge, Junction, MachineGroup } from "../state/types";

const PAD = 28;
const LABEL_H = 26;

export interface FloorPlatesProps {
  groups: MachineGroup[];
  /** Junction pucks occupy floors too — a junction-only floor earns a plate. */
  junctions: Junction[];
  edges: BeltEdge[];
  /** measured card geometry by node id (groups + junctions) */
  geoms: Record<string, { x: number; y: number; w: number; h: number }>;
  activeFloor: "all" | number;
}

export default function FloorPlates({ groups, junctions, edges, geoms, activeFloor }: FloorPlatesProps) {
  const plates = useMemo(() => {
    const byFloor = new Map<number, { groups: MachineGroup[]; junctions: Junction[] }>();
    const bucket = (floor: number) => {
      const b = byFloor.get(floor) ?? { groups: [], junctions: [] };
      byFloor.set(floor, b);
      return b;
    };
    for (const g of groups) bucket(g.floor).groups.push(g);
    for (const j of junctions) bucket(j.floor).junctions.push(j);
    if (byFloor.size < 2) return []; // single-floor factories stay clean

    // A boundary port has no floor — a group↔port belt is never a lift, so
    // a port end reads as the other end's floor (mirrors the edge renderer).
    const floorOf = (end: { kind: string; id: string }): number | null =>
      end.kind === "group"
        ? groups.find((g) => g.id === end.id)?.floor ?? 0
        : end.kind === "junction"
          ? junctions.find((j) => j.id === end.id)?.floor ?? 0
          : null;
    const endFloors = (e: BeltEdge): [number, number] => {
      const src = floorOf(e.from);
      const dst = floorOf(e.to);
      return [src ?? dst ?? 0, dst ?? src ?? 0];
    };

    return [...byFloor.entries()]
      .sort((a, b) => a[0] - b[0])
      .map(([floor, members]) => {
        let minX = Infinity;
        let minY = Infinity;
        let maxX = -Infinity;
        let maxY = -Infinity;
        for (const n of [...members.groups, ...members.junctions]) {
          const geom = geoms[n.id];
          if (!geom) continue;
          minX = Math.min(minX, geom.x);
          minY = Math.min(minY, geom.y);
          maxX = Math.max(maxX, geom.x + geom.w);
          maxY = Math.max(maxY, geom.y + geom.h);
        }
        if (!isFinite(minX)) return null;
        const floors = edges.map(endFloors);
        const liftsUp = floors.filter(([s, d]) => s === floor && d > floor).length;
        const liftsDown = floors.filter(([s, d]) => s === floor && d < floor).length;
        const liftsIn = floors.filter(([s, d]) => d === floor && s !== floor).length;
        return {
          floor,
          minX,
          minY,
          maxX,
          maxY,
          liftsUp,
          liftsDown,
          liftsIn,
          count: members.groups.length,
          junctionCount: members.junctions.length,
        };
      })
      .filter((p): p is NonNullable<typeof p> => p !== null);
  }, [groups, junctions, edges, geoms]);

  if (plates.length === 0) return null;

  return (
    <ViewportPortal>
      {plates.map((p) => {
        const active = activeFloor === "all" || activeFloor === p.floor;
        const liftBits: string[] = [];
        if (p.liftsUp) liftBits.push(`${p.liftsUp}⤒`);
        if (p.liftsDown) liftBits.push(`${p.liftsDown}⤓`);
        if (p.liftsIn) liftBits.push(`${p.liftsIn} IN`);
        const countBits: string[] = [];
        if (p.count > 0) countBits.push(`${p.count} ${p.count === 1 ? "GROUP" : "GROUPS"}`);
        if (p.junctionCount > 0)
          countBits.push(`${p.junctionCount} ${p.junctionCount === 1 ? "JUNCTION" : "JUNCTIONS"}`);
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
                {countBits.join(" · ")}
                {liftBits.length > 0 && ` · ${liftBits.join(" ")}`}
              </span>
            </div>
          </div>
        );
      })}
    </ViewportPortal>
  );
}
