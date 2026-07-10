// Boundary port cards (mock 4a): slim 200px cards at the factory edges
// carrying route context in from the map — Phase 1: node-claim ceilings for
// inputs, "TO WORLD" for outputs.

import { Handle, Position } from "@xyflow/react";
import { useStore } from "../state/store";
import { fmtRate } from "../lib/format";
import type { Port } from "../state/types";

export interface PortNodeData {
  port: Port;
  factoryId: string;
  [key: string]: unknown;
}

export default function BoundaryPortNode({ data, selected }: { data: PortNodeData; selected?: boolean }) {
  const { port } = data;
  const gamedata = useStore((s) => s.gamedata);
  const derived = useStore((s) => s.derived);
  const projected = useStore((s) => s.projected);
  const settled = useStore((s) => s.settled);

  const df = projected && projected.factoryId === port.factory ? projected.result : derived.factories[port.factory];
  const rate = df?.ports[port.id] ?? port.rate;
  const isProjected = (!!projected && projected.factoryId === port.factory) || port.status === "planned";
  const capped =
    port.direction === "in" && port.rateCeiling != null && rate >= port.rateCeiling - 1e-9 && rate > 0;
  const numCls = `${isProjected ? "projected" : ""} ${settled.has(`/ports/${port.id}`) ? "settle" : ""}`;
  const item = gamedata.items[port.item]?.displayName ?? port.item;

  return (
    <div
      className={`port-card ${port.direction} frame-${port.status} ${selected ? "selected" : ""}`}
      data-testid={`port-${port.direction}-${port.item}`}
    >
      <div className="port-card-dir t-label">{port.direction === "in" ? "INPUT" : "OUTPUT"}</div>
      <div className="port-card-item">
        <div className="icon-ph s20" />
        <span>{item}</span>
      </div>
      <div className={`t-data-12 ${numCls}`}>
        {fmtRate(rate)}
        <span className="unit">/min</span>
        {port.direction === "in" && port.rateCeiling != null && (
          <span className={`port-ceiling ${capped ? "capped" : ""}`}> / {fmtRate(port.rateCeiling)}</span>
        )}
      </div>
      <div className="port-card-src mono">{port.direction === "in" ? "FROM NODE CLAIM" : "TO WORLD"}</div>
      {port.direction === "in" ? (
        <Handle type="source" position={Position.Right} className="belt-handle" />
      ) : (
        <Handle type="target" position={Position.Left} className="belt-handle" />
      )}
    </div>
  );
}
