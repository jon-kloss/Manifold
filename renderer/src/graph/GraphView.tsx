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
import type { DerivedFactory, Id } from "../state/types";
import MachineGroupNode, { type GroupNodeData } from "./MachineGroupNode";
import BoundaryPortNode, { type PortNodeData } from "./BoundaryPortNode";
import BeltEdgeView, { type BeltEdgeData } from "./BeltEdgeView";
import Inspector from "./Inspector";
import RecipeStrip from "./RecipeStrip";
import AddGroupMenu from "./AddGroupMenu";
import AddPortMenu from "./AddPortMenu";
import { fmtPower } from "../lib/format";
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
  const { fitView } = useReactFlow();
  const [flowOverlay, setFlowOverlay] = useState(true);
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
      out.push({
        id: gid,
        type: "group",
        position: { x: g.graphPos.x, y: g.graphPos.y },
        data: { group: g, factoryId } satisfies GroupNodeData as unknown as Record<string, unknown>,
        selected: selection?.kind === "group" && selection.id === gid,
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
  }, [factory, plan.groups, plan.ports, selection, factoryId]);

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

  // ---- edges ----
  const edges: Edge[] = useMemo(() => {
    if (!factory) return [];
    return Object.values(plan.edges)
      .filter((e) => e.factory === factoryId)
      .map((e) => {
        const d = df?.edges[e.id];
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
          } satisfies BeltEdgeData as unknown as Record<string, unknown>,
        };
      });
  }, [factory, plan.edges, factoryId, df, selection, isProjected, flowOverlay, settled]);

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
