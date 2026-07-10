// Factory graph view (mock 4a): React Flow on a 16px dot grid. Boundary ports
// at the edges, machine-group cards between, flow-encoded belt edges. The
// solver contract (4c): every edit re-solves live; numbers change, geometry
// doesn't; infeasible hard-stops, never errors.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
  Background,
  BackgroundVariant,
  MiniMap,
  applyNodeChanges,
  type Connection,
  type Edge,
  type Node,
  type NodeChange,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { useStore, solveChip } from "../state/store";
import type { Command, DerivedFactory, Id } from "../state/types";
import MachineGroupNode, { type GroupNodeData } from "./MachineGroupNode";
import BoundaryPortNode, { type PortNodeData } from "./BoundaryPortNode";
import BeltEdgeView, { type BeltEdgeData } from "./BeltEdgeView";
import Inspector from "./Inspector";
import RecipeStrip from "./RecipeStrip";
import AddGroupMenu from "./AddGroupMenu";
import AddPortMenu from "./AddPortMenu";
import { fmtPower } from "../lib/format";
import { computeEdgeLayout, type LabelSize, type NodeGeom } from "./edgeLayout";
import FloorPlates from "./FloorPlates";
import { fmtRate, fmtPercent } from "../lib/format";
import { beltCapacity } from "../state/types";
import "./graph.css";

const nodeTypes = { group: MachineGroupNode, boundaryPort: BoundaryPortNode };
const edgeTypes = { belt: BeltEdgeView };

const snap = (v: number) => Math.round(v / 16) * 16;

function GraphViewInner({ factoryId }: { factoryId: Id }) {
  const plan = useStore((s) => s.plan);
  const derived = useStore((s) => s.derived);
  const projected = useStore((s) => s.projected);
  const selection = useStore((s) => s.selection);
  const settled = useStore((s) => s.settled);
  const setSelection = useStore((s) => s.setSelection);
  const setView = useStore((s) => s.setView);
  const dispatch = useStore((s) => s.dispatch);

  const factory = plan.factories[factoryId];
  const { fitView, getNodes } = useReactFlow();
  const [flowOverlay, setFlowOverlay] = useState(true);
  // Floor filter: 'all' or a specific floor. Chips appear once floors exist.
  const [floorFilter, setFloorFilter] = useState<"all" | number>("all");
  const floors = useMemo(() => {
    const set = new Set<number>([0]);
    for (const gid of factory?.groups ?? []) {
      const g = plan.groups[gid];
      if (g) set.add(g.floor);
    }
    return [...set].sort((a, b) => a - b);
  }, [factory, plan.groups]);
  const groupFloor = useCallback(
    (id: string): number => useStore.getState().plan.groups[id]?.floor ?? 0,
    [],
  );
  const jumpFloor = useCallback(
    (floor: number, edgeId: string) => {
      setFloorFilter(floor);
      setSelection({ kind: "edge", id: edgeId });
    },
    [setSelection],
  );

  /** Band-stacking core: move each floor's cards into its own horizontal band
   *  (highest floor on top), preserving intra-floor layout. `floorOf` lets
   *  AUTO-FLOOR stack against floors it is about to assign. */
  const bandMoves = useCallback(
    (groups: { id: string; graphPos: { x: number; y: number } }[], floorOf: (id: string) => number): Command[] => {
      const byFloor = new Map<number, typeof groups>();
      for (const g of groups) {
        const fl = floorOf(g.id);
        byFloor.set(fl, [...(byFloor.get(fl) ?? []), g]);
      }
      if (byFloor.size < 2) return [];
      const measured: Record<string, { y: number; h: number }> = {};
      for (const n of getNodes()) {
        const m = (n as { measured?: { height?: number } }).measured;
        measured[n.id] = { y: n.position.y, h: m?.height ?? 150 };
      }
      const GAP = 96;
      const snap16 = (v: number) => Math.round(v / 16) * 16;
      const floorsDesc = [...byFloor.keys()].sort((a, b) => b - a);
      // anchor the stack where the plan already lives
      let cursorY = Math.min(...groups.map((g) => measured[g.id]?.y ?? g.graphPos.y));
      const cmds: Command[] = [];
      for (const floor of floorsDesc) {
        const members = byFloor.get(floor)!;
        const tops = members.map((g) => measured[g.id]?.y ?? g.graphPos.y);
        const bottoms = members.map((g) => (measured[g.id]?.y ?? g.graphPos.y) + (measured[g.id]?.h ?? 150));
        const minY = Math.min(...tops);
        const bandH = Math.max(...bottoms) - minY;
        for (const g of members) {
          const newY = snap16(cursorY + ((measured[g.id]?.y ?? g.graphPos.y) - minY));
          if (Math.abs(newY - g.graphPos.y) > 0.5) {
            cmds.push({ type: "move_group_card", id: g.id, graphPos: { x: g.graphPos.x, y: newY } });
          }
        }
        cursorY += bandH + GAP;
      }
      return cmds;
    },
    [getNodes],
  );

  const commitArrange = useCallback(
    (cmds: Command[]) => {
      if (cmds.length === 0) return;
      void dispatch(cmds).then(() => {
        window.setTimeout(() => void fitView({ padding: 0.15, duration: 300 }), 60);
      });
    },
    [dispatch, fitView],
  );

  /** Cutaway elevation from the floors as they stand. One undo step. */
  const stackFloors = useCallback(() => {
    const state = useStore.getState();
    const f = state.plan.factories[factoryId];
    if (!f) return;
    const groups = f.groups.map((gid) => state.plan.groups[gid]).filter(Boolean);
    commitArrange(bandMoves(groups, (id) => state.plan.groups[id]?.floor ?? 0));
  }, [factoryId, bandMoves, commitArrange]);

  /** Assign floors by production stage — topological depth from the input
   *  side (smelting low, final assembly high) — then band-stack. One undo step. */
  const autoFloor = useCallback(() => {
    const state = useStore.getState();
    const f = state.plan.factories[factoryId];
    if (!f) return;
    const groups = f.groups.map((gid) => state.plan.groups[gid]).filter(Boolean);
    if (groups.length < 2) return;
    const preds = new Map<string, string[]>();
    for (const e of Object.values(state.plan.edges)) {
      if (e.factory === factoryId && e.from.kind === "group" && e.to.kind === "group") {
        preds.set(e.to.id, [...(preds.get(e.to.id) ?? []), e.from.id]);
      }
    }
    const stage = new Map<string, number>();
    const visiting = new Set<string>();
    let cyclic = false;
    const depth = (id: string): number => {
      if (stage.has(id)) return stage.get(id)!;
      if (visiting.has(id)) {
        cyclic = true;
        return 0;
      }
      visiting.add(id);
      const ps = preds.get(id) ?? [];
      const d = ps.length ? Math.max(...ps.map(depth)) + 1 : 0;
      visiting.delete(id);
      stage.set(id, d);
      return d;
    };
    groups.forEach((g) => depth(g.id));
    if (cyclic) return; // loops have no stages — leave the plan alone

    const cmds: Command[] = [];
    for (const g of groups) {
      const fl = stage.get(g.id) ?? 0;
      if (fl !== g.floor) cmds.push({ type: "set_group_floor", id: g.id, floor: fl });
    }
    cmds.push(...bandMoves(groups, (id) => stage.get(id) ?? 0));
    commitArrange(cmds);
  }, [factoryId, bandMoves, commitArrange]);
  const [addMenu, setAddMenu] = useState<{ x: number; y: number; flowX: number; flowY: number } | null>(null);
  const [portMenu, setPortMenu] = useState<"in" | "out" | null>(null);

  // Display derived: T0 projection during drag, else authoritative T1.
  const df: DerivedFactory | undefined =
    projected && projected.factoryId === factoryId ? projected.result : derived.factories[factoryId];
  const isProjected = !!projected && projected.factoryId === factoryId;

  // ---- nodes (positions locally tracked while dragging; committed on drop) ----
  const buildNodes = useCallback((): Node[] => {
    if (!factory) return [];
    const out: Node[] = [];
    for (const gid of factory.groups) {
      const g = plan.groups[gid];
      if (!g) continue;
      const dimmed = floorFilter !== "all" && g.floor !== floorFilter;
      out.push({
        id: gid,
        type: "group",
        position: { x: g.graphPos.x, y: g.graphPos.y },
        data: { group: g, factoryId, showFloorBadge: floors.length > 1 } satisfies GroupNodeData as unknown as Record<string, unknown>,
        selected: selection?.kind === "group" && selection.id === gid,
        // ghosts of other floors: visible context, but never interactive
        style: dimmed ? { opacity: 0.22, pointerEvents: "none" as const } : undefined,
      });
    }
    for (const pid of factory.ports) {
      const p = plan.ports[pid];
      if (!p) continue;
      out.push({
        id: pid,
        type: "boundaryPort",
        position: { x: p.graphPos.x, y: p.graphPos.y },
        data: { port: p, factoryId } satisfies PortNodeData as unknown as Record<string, unknown>,
        selected: selection?.kind === "port" && selection.id === pid,
      });
    }
    return out;
  }, [factory, plan.groups, plan.ports, selection, factoryId, floorFilter, floors.length]);

  const [nodes, setNodes] = useState<Node[]>(buildNodes);
  useEffect(() => setNodes(buildNodes()), [buildNodes]);

  const onNodesChange = useCallback(
    (changes: NodeChange[]) => {
      setNodes((ns) => applyNodeChanges(changes, ns));
      for (const ch of changes) {
        if (ch.type === "select" && ch.selected) {
          const isGroup = !!useStore.getState().plan.groups[ch.id];
          setSelection(isGroup ? { kind: "group", id: ch.id } : { kind: "port", id: ch.id });
        }
        if (ch.type === "position" && ch.dragging === false) {
          const node = useStore.getState().plan.groups[ch.id]
            ? ("group" as const)
            : ("port" as const);
          const current = nodes.find((n) => n.id === ch.id);
          const pos = ch.position ?? current?.position;
          if (!pos) continue;
          const graphPos = { x: snap(pos.x), y: snap(pos.y) };
          void dispatch([
            node === "group"
              ? { type: "move_group_card", id: ch.id, graphPos }
              : { type: "move_port_card", id: ch.id, graphPos },
          ]);
        }
      }
    },
    [dispatch, setSelection, nodes],
  );

  // ---- edges (belt-style layout: shared anchors, orthogonal runs, hops) ----
  const edges: Edge[] = useMemo(() => {
    if (!factory) return [];
    const beltEdges = Object.values(plan.edges).filter((e) => e.factory === factoryId);
    const geoms: Record<string, NodeGeom> = {};
    for (const n of nodes) {
      const m = (n as { measured?: { width?: number; height?: number } }).measured;
      geoms[n.id] = {
        x: n.position.x,
        y: n.position.y,
        w: m?.width ?? (n.type === "group" ? 248 : 200),
        h: m?.height ?? (n.type === "group" ? 150 : 96),
      };
    }
    // Chip footprint from the real text (mono ≈ 6.4px/char at 10px + padding).
    const labelSizes: Record<string, LabelSize> = {};
    for (const e of beltEdges) {
      const d = df?.edges[e.id];
      const text = `${fmtRate(d?.flow ?? 0)}/${fmtRate(beltCapacity(e.tier))} · ${fmtPercent(d?.saturation ?? 0)} MK.${e.tier}`;
      labelSizes[e.id] = { w: text.length * 6.4 + 16, h: 20 };
    }
    const layout = computeEdgeLayout(
      geoms,
      beltEdges.map((e) => ({ id: e.id, source: e.from.id, target: e.to.id })),
      labelSizes,
    );
    // Portal stubs: on a filtered floor, a cross-floor belt runs from its
    // on-floor card to a lift portal instead of dimming into noise. Stubs on
    // the same card face fan out so several lifts stay distinct.
    const portalCounts = new Map<string, number>();
    return beltEdges.map((e) => {
      const d = df?.edges[e.id];
      const srcFloor = e.from.kind === "group" ? groupFloor(e.from.id) : 0;
      const dstFloor = e.to.kind === "group" ? groupFloor(e.to.id) : 0;
      const lift = srcFloor !== dstFloor;
      let dimmed = floorFilter !== "all" && srcFloor !== floorFilter && dstFloor !== floorFilter;
      let geom = layout[e.id] ?? null;
      let portal: { x: number; y: number; dir: "up" | "down"; otherFloor: number } | null = null;
      if (floorFilter !== "all" && lift) {
        const srcOn = srcFloor === floorFilter;
        const dstOn = dstFloor === floorFilter;
        if (srcOn !== dstOn) {
          const anchorNode = geoms[srcOn ? e.from.id : e.to.id];
          if (anchorNode) {
            const key = `${srcOn ? e.from.id : e.to.id}:${srcOn ? "out" : "in"}`;
            const idx = portalCounts.get(key) ?? 0;
            portalCounts.set(key, idx + 1);
            const y = anchorNode.y + anchorNode.h / 2 + (idx % 2 === 0 ? 1 : -1) * Math.ceil(idx / 2) * 26;
            const fromX = srcOn ? anchorNode.x + anchorNode.w : anchorNode.x;
            const toX = srcOn ? fromX + 72 : fromX - 72;
            const otherFloor = srcOn ? dstFloor : srcFloor;
            portal = { x: toX, y, dir: otherFloor > floorFilter ? "up" : "down", otherFloor };
            geom = {
              points: [],
              hops: [],
              path: `M ${fromX} ${y} L ${toX} ${y}`,
              labelX: toX,
              labelY: y,
              pathLen: 72,
            };
            dimmed = false;
          }
        }
      }
      return {
        id: e.id,
        source: e.from.id,
        target: e.to.id,
        type: "belt",
        selected: selection?.kind === "edge" && selection.id === e.id,
        data: {
          edge: e,
          flow: d?.flow ?? 0,
          saturation: d?.saturation ?? 0,
          projected: isProjected || e.status === "planned",
          flowOverlay,
          settled: settled.has(`/edges/${e.id}`),
          geom,
          lift,
          srcFloor,
          dstFloor,
          portal,
          onJumpFloor: jumpFloor,
          dimmed,
        } satisfies BeltEdgeData as unknown as Record<string, unknown>,
      };
    });
  }, [factory, plan.edges, factoryId, df, selection, isProjected, flowOverlay, settled, nodes, floorFilter, groupFloor, jumpFloor]);

  // Card geometry for the floor plates (same source as the edge layout).
  const plateGeoms = useMemo(() => {
    const out: Record<string, NodeGeom> = {};
    for (const n of nodes) {
      if (n.type !== "group") continue;
      const m = (n as { measured?: { width?: number; height?: number } }).measured;
      out[n.id] = { x: n.position.x, y: n.position.y, w: m?.width ?? 248, h: m?.height ?? 150 };
    }
    return out;
  }, [nodes]);
  const factoryGroups = useMemo(
    () => (factory ? factory.groups.map((gid) => plan.groups[gid]).filter(Boolean) : []),
    [factory, plan.groups],
  );
  const factoryEdges = useMemo(
    () => Object.values(plan.edges).filter((e) => e.factory === factoryId),
    [plan.edges, factoryId],
  );

  const onConnect = useCallback(
    (conn: Connection) => {
      const state = useStore.getState();
      const { plan: p, gamedata: gd } = state;
      // Infer the item: source's produced items ∩ target's consumed items.
      const produced = (id: string): string[] => {
        const g = p.groups[id];
        if (g) return (gd.recipes[g.recipe]?.products ?? []).map(([item]) => item);
        const port = p.ports[id];
        return port && port.direction === "in" ? [port.item] : [];
      };
      const consumed = (id: string): string[] => {
        const g = p.groups[id];
        if (g) return (gd.recipes[g.recipe]?.ingredients ?? []).map(([item]) => item);
        const port = p.ports[id];
        return port && port.direction === "out" ? [port.item] : [];
      };
      if (!conn.source || !conn.target) return;
      const item = produced(conn.source).find((i) => consumed(conn.target!).includes(i));
      if (!item) return; // no shared item — connection is meaningless, refuse silently
      const from = p.groups[conn.source] ? { kind: "group" as const, id: conn.source } : { kind: "port" as const, id: conn.source };
      const to = p.groups[conn.target] ? { kind: "group" as const, id: conn.target } : { kind: "port" as const, id: conn.target };
      void dispatch([{ type: "add_edge", factory: factoryId, from, to, item, tier: 1 }]);
    },
    [dispatch, factoryId],
  );

  // ---- keys: ESC world, ⌫ delete planned, R recipes ----
  const [stripOpen, setStripOpen] = useState(true);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement) return;
      if (e.key === "Escape") {
        if (addMenu) setAddMenu(null);
        else if (selection) setSelection(null);
        else setView({ mode: "map" });
      } else if (e.key === "Backspace" || e.key === "Delete") {
        const sel = useStore.getState().selection;
        if (!sel) return;
        if (sel.kind === "group") void dispatch([{ type: "delete_group", id: sel.id }]);
        else if (sel.kind === "edge") void dispatch([{ type: "delete_edge", id: sel.id }]);
        else if (sel.kind === "port") void dispatch([{ type: "delete_port", id: sel.id }]);
        setSelection(null);
      } else if (e.key === "r" || e.key === "R") {
        setStripOpen((o) => !o);
      } else if (e.key === "f" || e.key === "F") {
        void fitView({ padding: 0.15, duration: 200 });
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selection, addMenu, dispatch, setSelection, setView, fitView]);

  const flowRef = useRef<HTMLDivElement>(null);

  if (!factory) {
    // Factory deleted while open — return to the map, no dead end.
    setView({ mode: "map" });
    return null;
  }

  const chip = solveChip(derived.factories[factoryId]);
  const selectedGroup = selection?.kind === "group" ? plan.groups[selection.id] : null;
  const selectedPort = selection?.kind === "port" ? plan.ports[selection.id] : null;
  const statusGlyph = factory.status === "planned" ? "◇" : factory.status === "built" ? "◆" : "◈";

  return (
    <div className="graph-root" data-testid="graph-root">
      {/* context bar (36px) */}
      <div className="graph-contextbar">
        <button className="chip ctx-back" onClick={() => setView({ mode: "map" })} data-testid="btn-world">
          ⟵ WORLD · ESC
        </button>
        <span className="t-panel-header">{factory.name.toUpperCase()}</span>
        <span className={`chip ${factory.status === "planned" ? "planned" : ""}`}>
          {statusGlyph} {factory.status.replace("_", " ").toUpperCase()}
        </span>
        <span className={`chip ${isProjected ? "planned" : ""}`} data-testid="ctx-power">
          {fmtPower(df?.totalPowerMw ?? 0)}
        </span>
        <span className={`chip ${chip.over ? "warn" : ""}`}>{chip.text}</span>
        {df?.solveOnRelease && <span className="chip warn">LIVE → ON RELEASE</span>}
        <span className="ctx-spring" />
        <button
          className="btn btn-ghost overlay-chip"
          onClick={autoFloor}
          title="Assign floors by production stage (inputs low, assembly high) and stack — one undo step"
          data-testid="btn-auto-floor"
        >
          AUTO-FLOOR
        </button>
        {floors.length > 1 && (
          <button
            className="btn btn-ghost overlay-chip"
            onClick={stackFloors}
            title="Arrange each floor into its own band — highest floor on top, one undo step"
            data-testid="btn-stack-floors"
          >
            STACK FLOORS
          </button>
        )}
        {floors.length > 1 && (
          <div className="floor-chips" data-testid="floor-chips">
            <button
              className={`btn btn-ghost overlay-chip ${floorFilter === "all" ? "active" : ""}`}
              onClick={() => setFloorFilter("all")}
            >
              ALL
            </button>
            {floors.map((f) => (
              <button
                key={f}
                className={`btn btn-ghost overlay-chip ${floorFilter === f ? "active" : ""}`}
                onClick={() => setFloorFilter(floorFilter === f ? "all" : f)}
              >
                F{f}
              </button>
            ))}
          </div>
        )}
        <button
          className={`btn btn-ghost overlay-chip ${flowOverlay ? "active" : ""}`}
          onClick={() => setFlowOverlay(!flowOverlay)}
        >
          FLOW
        </button>
        <button className="btn btn-ghost" onClick={() => setPortMenu("in")}>
          + IN PORT
        </button>
        <button className="btn btn-ghost" onClick={() => setPortMenu("out")}>
          + OUT PORT
        </button>
      </div>

      <div
        className="graph-canvas"
        ref={flowRef}
        onDoubleClick={(e) => {
          // dblclick canvas = add group (4c)
          const target = e.target as HTMLElement;
          if (!target.classList.contains("react-flow__pane")) return;
          const rect = flowRef.current!.getBoundingClientRect();
          setAddMenu({ x: e.clientX - rect.left, y: e.clientY - rect.top, flowX: e.clientX, flowY: e.clientY });
        }}
      >
        <ReactFlow
          nodes={nodes}
          edges={edges}
          nodeTypes={nodeTypes}
          edgeTypes={edgeTypes}
          onNodesChange={onNodesChange}
          onConnect={onConnect}
          onEdgeClick={(_, edge) => setSelection({ kind: "edge", id: edge.id })}
          onPaneClick={() => {
            setSelection(null);
            setAddMenu(null);
          }}
          zoomOnDoubleClick={false}
          snapToGrid
          snapGrid={[16, 16]}
          minZoom={0.25}
          maxZoom={2}
          fitView
          proOptions={{ hideAttribution: true }}
          deleteKeyCode={null}
        >
          <Background variant={BackgroundVariant.Dots} gap={16} size={1.5} color="var(--graph-dot)" />
          <FloorPlates groups={factoryGroups} edges={factoryEdges} geoms={plateGeoms} activeFloor={floorFilter} />
          <MiniMap
            position="bottom-left"
            className="graph-minimap"
            pannable
            zoomable
            nodeColor={() => "var(--steel-600)"}
            maskColor="rgba(13,16,19,.7)"
            bgColor="var(--steel-950)"
          />
        </ReactFlow>
        <div className="minimap-caption mono">ESC ⟶ WORLD</div>
      </div>

      {addMenu && (
        <AddGroupMenu
          at={addMenu}
          factoryId={factoryId}
          floor={floorFilter === "all" ? 0 : floorFilter}
          onClose={() => setAddMenu(null)}
          flowRef={flowRef}
        />
      )}
      {portMenu && <AddPortMenu direction={portMenu} factoryId={factoryId} onClose={() => setPortMenu(null)} />}

      {(selectedGroup || selectedPort || selection?.kind === "edge") && (
        <Inspector factoryId={factoryId} df={df} isProjected={isProjected} />
      )}

      {selectedGroup && stripOpen && <RecipeStrip group={selectedGroup} />}
    </div>
  );
}

export default function GraphView({ factoryId }: { factoryId: Id }) {
  return (
    <ReactFlowProvider>
      <GraphViewInner factoryId={factoryId} />
    </ReactFlowProvider>
  );
}
