// Factory graph view (mock 4a): React Flow on a 16px dot grid. Boundary ports
// at the edges, machine-group cards between, flow-encoded belt edges. The
// solver contract (4c): every edit re-solves live; numbers change, geometry
// doesn't; infeasible hard-stops, never errors.

import { useCallback, useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
import {
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
  Background,
  BackgroundVariant,
  MiniMap,
  SelectionMode,
  applyNodeChanges,
  type Connection,
  type Edge,
  type Node,
  type NodeChange,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { useStore, solveChip, errText } from "../state/store";
import type { Command, DerivedFactory, Id } from "../state/types";
import MachineGroupNode, { type GroupNodeData } from "./MachineGroupNode";
import BoundaryPortNode, { type PortNodeData } from "./BoundaryPortNode";
import JunctionNode, { type JunctionNodeData } from "./JunctionNode";
import BeltEdgeView, { type BeltEdgeData } from "./BeltEdgeView";
import Inspector from "./Inspector";
import RecipeStrip from "./RecipeStrip";
import AddGroupMenu from "./AddGroupMenu";
import AddPortMenu from "./AddPortMenu";
import BuildSheet from "./BuildSheet";
import MakeFromResources from "./MakeFromResources";
import GraphContextMenu, { type CtxTarget } from "./GraphContextMenu";
import { fmtPower } from "../lib/format";
import ItemIcon from "../lib/ItemIcon";
import { isEditableTarget } from "../lib/keys";
import { computeEdgeLayout, type JunctionShape, type LabelSize, type NodeGeom } from "./edgeLayout";
import FloorPlates from "./FloorPlates";
import { fmtRate, fmtPercent, bottleneckEdges } from "../lib/format";
import { beltCapacity } from "../state/types";
import "./graph.css";

const nodeTypes = { group: MachineGroupNode, boundaryPort: BoundaryPortNode, junction: JunctionNode };
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
  const setReviewing = useStore((s) => s.setReviewing);
  const [t2Busy, setT2Busy] = useState(false);
  const [t2Note, setT2Note] = useState<string | null>(null);

  // T2 (SDD §5.3): factory-scoped recipe optimization → mini-proposal.
  const runT2 = async () => {
    setT2Busy(true);
    setT2Note(null);
    try {
      const { backend } = await import("../state/backend");
      const proposal = await backend.t2Optimize(factoryId);
      if (!proposal) {
        setT2Note("NO CHEAPER RECIPES FOUND");
        setTimeout(() => setT2Note(null), 3000);
        return;
      }
      const created = await dispatch([{ type: "create_proposal", proposal }]);
      const id = created?.[0];
      if (id) {
        setView({ mode: "map" });
        setReviewing(id);
      }
    } catch (e) {
      useStore.getState().reportCmdError(errText(e));
    } finally {
      setT2Busy(false);
    }
  };

  const factory = plan.factories[factoryId];
  const { fitView, getNodes, screenToFlowPosition: screenToFlow } = useReactFlow();
  const [flowOverlay, setFlowOverlay] = useState(true);
  // Floor filter: 'all' or a specific floor. Chips appear once floors exist.
  const [floorFilter, setFloorFilter] = useState<"all" | number>("all");
  const floors = useMemo(() => {
    const set = new Set<number>([0]);
    for (const gid of factory?.groups ?? []) {
      const g = plan.groups[gid];
      if (g) set.add(g.floor);
    }
    for (const j of Object.values(plan.junctions)) {
      if (j.factory === factoryId) set.add(j.floor);
    }
    return [...set].sort((a, b) => a - b);
  }, [factory, plan.groups, plan.junctions, factoryId]);
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
    (
      groups: { id: string; graphPos: { x: number; y: number }; junction?: boolean }[],
      floorOf: (id: string) => number,
    ): Command[] => {
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
            cmds.push(
              g.junction
                ? { type: "move_junction_card", id: g.id, graphPos: { x: g.graphPos.x, y: newY } }
                : { type: "move_group_card", id: g.id, graphPos: { x: g.graphPos.x, y: newY } },
            );
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
      void dispatch(cmds).then((r) => {
        if (r) window.setTimeout(() => void fitView({ padding: 0.15, duration: 300 }), 60);
      });
    },
    [dispatch, fitView],
  );

  /** Cutaway elevation from the floors as they stand. One undo step. */
  const stackFloors = useCallback(() => {
    const state = useStore.getState();
    const f = state.plan.factories[factoryId];
    if (!f) return;
    const placeables = [
      ...f.groups.map((gid) => state.plan.groups[gid]).filter(Boolean),
      ...Object.values(state.plan.junctions)
        .filter((j) => j.factory === factoryId)
        .map((j) => ({ ...j, junction: true })),
    ];
    commitArrange(
      bandMoves(placeables, (id) => state.plan.groups[id]?.floor ?? state.plan.junctions[id]?.floor ?? 0),
    );
  }, [factoryId, bandMoves, commitArrange]);

  /** Assign floors by production stage — topological depth from the input
   *  side (smelting low, final assembly high) — then band-stack. One undo step. */
  const autoFloor = useCallback(() => {
    const state = useStore.getState();
    const f = state.plan.factories[factoryId];
    if (!f) return;
    const groups = f.groups.map((gid) => state.plan.groups[gid]).filter(Boolean);
    const junctions = Object.values(state.plan.junctions).filter((j) => j.factory === factoryId);
    if (groups.length < 2) return;
    // stage over groups AND junctions so belts through splitters/mergers count
    const staged = (k: string) => k === "group" || k === "junction";
    const preds = new Map<string, string[]>();
    for (const e of Object.values(state.plan.edges)) {
      if (e.factory === factoryId && staged(e.from.kind) && staged(e.to.kind)) {
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
    junctions.forEach((j) => depth(j.id));
    if (cyclic) return; // loops have no stages — leave the plan alone

    // a junction sits on its feeder's floor (it's part of that floor's belt run)
    const junctionStage = (id: string) => Math.max(0, (stage.get(id) ?? 1) - 1);
    const cmds: Command[] = [];
    for (const g of groups) {
      const fl = stage.get(g.id) ?? 0;
      if (fl !== g.floor) cmds.push({ type: "set_group_floor", id: g.id, floor: fl });
    }
    for (const j of junctions) {
      const fl = junctionStage(j.id);
      if (fl !== j.floor) cmds.push({ type: "set_junction_floor", id: j.id, floor: fl });
    }
    cmds.push(
      ...bandMoves(
        [...groups, ...junctions.map((j) => ({ ...j, junction: true }))],
        (id) => (state.plan.junctions[id] ? junctionStage(id) : stage.get(id) ?? 0),
      ),
    );
    commitArrange(cmds);
  }, [factoryId, bandMoves, commitArrange]);
  const [addMenu, setAddMenu] = useState<{ x: number; y: number; flowX: number; flowY: number } | null>(null);
  const [portMenu, setPortMenu] = useState<"in" | "out" | null>(null);
  const [logisticMenu, setLogisticMenu] = useState(false);
  const [buildSheet, setBuildSheet] = useState(false);
  const [makeOpen, setMakeOpen] = useState(false);
  const [ctx, setCtx] = useState<CtxTarget | null>(null);
  // True while a marquee box-selection is in progress — suppresses the
  // single-selection sync so React Flow keeps the whole multi-selection.
  const boxSelRef = useRef(false);

  // Right-click a node → context menu over the current selection if this node
  // is part of it (bulk), else just this node.
  const openNodeCtx = useCallback(
    (e: ReactMouseEvent, node: Node) => {
      e.preventDefault();
      const selected = getNodes().filter((n) => n.selected).map((n) => n.id);
      const nodeIds = node.selected && selected.length > 1 ? selected : [node.id];
      setCtx({ x: e.clientX, y: e.clientY, nodeIds });
    },
    [getNodes],
  );
  const openSelectionCtx = useCallback((e: ReactMouseEvent, sel: Node[]) => {
    e.preventDefault();
    setCtx({ x: e.clientX, y: e.clientY, nodeIds: sel.map((n) => n.id) });
  }, []);
  // Clicking one member of a box-selection narrows to just it. React Flow emits
  // no select change for a plain click on an already-selected node, so drive the
  // collapse here or the marquee would stay stuck with no inspector.
  const narrowOnClick = useCallback(
    (_: ReactMouseEvent, node: Node) => {
      if (getNodes().filter((n) => n.selected).length <= 1) return;
      boxSelRef.current = false;
      setNodes((ns) => ns.map((n) => ({ ...n, selected: n.id === node.id })));
      const st = useStore.getState().plan;
      setSelection(
        st.groups[node.id]
          ? { kind: "group", id: node.id }
          : st.junctions[node.id]
            ? { kind: "junction", id: node.id }
            : { kind: "port", id: node.id },
      );
    },
    [getNodes, setSelection],
  );

  // Display derived: T0 projection during drag, else authoritative T1.
  const df: DerivedFactory | undefined =
    projected && projected.factoryId === factoryId ? projected.result : derived.factories[factoryId];
  const isProjected = !!projected && projected.factoryId === factoryId;

  // Trace-on-select: when a machine/junction/port is selected, the set of nodes
  // reachable from it through belt edges (both directions) — its whole connected
  // production chain. Everything OUTSIDE this set dims so the flow reads at a
  // glance. `null` when nothing traceable is selected (no dimming).
  const traceSet = useMemo<Set<string> | null>(() => {
    if (!factory) return null;
    if (selection?.kind !== "group" && selection?.kind !== "junction" && selection?.kind !== "port") {
      return null;
    }
    const adj = new Map<string, string[]>();
    const link = (a: string, b: string) => {
      (adj.get(a) ?? adj.set(a, []).get(a)!).push(b);
    };
    for (const e of Object.values(plan.edges)) {
      if (e.factory !== factoryId) continue;
      link(e.from.id, e.to.id);
      link(e.to.id, e.from.id);
    }
    const seen = new Set<string>([selection.id]);
    const queue = [selection.id];
    while (queue.length) {
      const cur = queue.shift()!;
      for (const nb of adj.get(cur) ?? []) {
        if (!seen.has(nb)) {
          seen.add(nb);
          queue.push(nb);
        }
      }
    }
    // An isolated selection (nothing wired to it) traces only itself — no point
    // dimming the whole graph for a single unconnected card.
    return seen.size > 1 ? seen : null;
  }, [factory, selection, plan.edges, factoryId]);

  // ---- nodes (positions locally tracked while dragging; committed on drop) ----
  const buildNodes = useCallback((): Node[] => {
    if (!factory) return [];
    const out: Node[] = [];
    for (const gid of factory.groups) {
      const g = plan.groups[gid];
      if (!g) continue;
      const dimmed = floorFilter !== "all" && g.floor !== floorFilter;
      const traceDim = !!traceSet && !traceSet.has(gid);
      out.push({
        id: gid,
        type: "group",
        position: { x: g.graphPos.x, y: g.graphPos.y },
        data: { group: g, factoryId, showFloorBadge: floors.length > 1 } satisfies GroupNodeData as unknown as Record<string, unknown>,
        selected: selection?.kind === "group" && selection.id === gid,
        // ghosts of other floors: visible context, but never interactive. Trace
        // dimming (off-chain when something is selected) stays clickable so you
        // can hop along the chain.
        style: dimmed
          ? { opacity: 0.22, pointerEvents: "none" as const }
          : traceDim
            ? { opacity: 0.3 }
            : undefined,
      });
    }
    for (const j of Object.values(plan.junctions)) {
      if (j.factory !== factoryId) continue;
      const dimmed = floorFilter !== "all" && j.floor !== floorFilter;
      const traceDim = !!traceSet && !traceSet.has(j.id);
      out.push({
        id: j.id,
        type: "junction",
        position: { x: j.graphPos.x, y: j.graphPos.y },
        data: { junction: j, factoryId, showFloorBadge: floors.length > 1 } satisfies JunctionNodeData as unknown as Record<string, unknown>,
        selected: selection?.kind === "junction" && selection.id === j.id,
        style: dimmed
          ? { opacity: 0.22, pointerEvents: "none" as const }
          : traceDim
            ? { opacity: 0.3 }
            : undefined,
      });
    }
    for (const pid of factory.ports) {
      const p = plan.ports[pid];
      if (!p) continue;
      const traceDim = !!traceSet && !traceSet.has(pid);
      out.push({
        id: pid,
        type: "boundaryPort",
        position: { x: p.graphPos.x, y: p.graphPos.y },
        data: { port: p, factoryId } satisfies PortNodeData as unknown as Record<string, unknown>,
        selected: selection?.kind === "port" && selection.id === pid,
        style: traceDim ? { opacity: 0.3 } : undefined,
      });
    }
    return out;
  }, [factory, plan.groups, plan.ports, plan.junctions, selection, factoryId, floorFilter, floors.length, traceSet]);

  const [nodes, setNodes] = useState<Node[]>(buildNodes);
  // Plan/selection changes rebuild the node array — but xyflow adopts user
  // nodes with checkEquality: an UNCHANGED OBJECT REFERENCE skips the rebuild
  // of that node's internals entirely (measured dims, handleBounds). The old
  // blanket `setNodes(buildNodes())` handed xyflow all-new objects without
  // `measured`, so every EdgeWrapper returned null for the re-measure window
  // (~190-430 ms) — unmounting every belt edge and restarting all
  // .edge-flowing dash animations at phase 0 on any click or edit. So:
  // - reuse the PREVIOUS node object when nothing changed. Plan objects are
  //   immutably replaced upstream, so comparing data payloads by REFERENCE
  //   (prev.data.group === fresh.data.group, …) is a valid cheap check;
  // - when something DID change, return the fresh node carrying prev.measured
  //   so handleBounds survives and edges never hit the null window.
  // Everything keys off the setNodes callback's `prev` (never a side ref) —
  // it must see the dimension/position updates applyNodeChanges folded in.
  useEffect(() => {
    setNodes((prev) => {
      const fresh = buildNodes();
      const prevById = new Map(prev.map((n) => [n.id, n]));
      // While a marquee selection is live (or several nodes are already
      // box-selected), React Flow owns `selected` — this single-selection
      // rebuild must not collapse it. Preserve each node's current selected.
      const multi = boxSelRef.current || prev.filter((n) => n.selected).length > 1;
      let identical = fresh.length === prev.length;
      const next = fresh.map((f, i) => {
        const p = prevById.get(f.id);
        if (!p) {
          identical = false;
          return f;
        }
        const fSelected = multi ? !!p.selected : !!f.selected;
        if (multi) f = { ...f, selected: fSelected };
        const pd = p.data as Record<string, unknown>;
        const fd = f.data as Record<string, unknown>;
        const unchanged =
          p.type === f.type &&
          !!p.selected === fSelected &&
          p.position.x === f.position.x &&
          p.position.y === f.position.y &&
          p.style?.opacity === f.style?.opacity &&
          p.style?.pointerEvents === f.style?.pointerEvents &&
          pd.group === fd.group &&
          pd.junction === fd.junction &&
          pd.port === fd.port &&
          pd.factoryId === fd.factoryId &&
          pd.showFloorBadge === fd.showFloorBadge;
        if (unchanged) {
          if (p !== prev[i]) identical = false;
          return p;
        }
        identical = false;
        return { ...f, measured: p.measured };
      });
      // nothing changed at all → keep the previous array (skip the re-render)
      return identical ? prev : next;
    });
  }, [buildNodes]);

  const onNodesChange = useCallback(
    (changes: NodeChange[]) => {
      setNodes((ns) => applyNodeChanges(changes, ns));
      for (const ch of changes) {
        // Skip the single-selection sync during a marquee drag — otherwise each
        // node entering the box would collapse the app selection to just it and
        // the rebuild would drop the rest of the box-selection.
        if (ch.type === "select" && ch.selected && !boxSelRef.current) {
          const st = useStore.getState().plan;
          setSelection(
            st.groups[ch.id]
              ? { kind: "group", id: ch.id }
              : st.junctions[ch.id]
                ? { kind: "junction", id: ch.id }
                : { kind: "port", id: ch.id },
          );
        }
        if (ch.type === "position" && ch.dragging === false) {
          const st = useStore.getState().plan;
          const current = nodes.find((n) => n.id === ch.id);
          const pos = ch.position ?? current?.position;
          if (!pos) continue;
          const graphPos = { x: snap(pos.x), y: snap(pos.y) };
          void dispatch([
            st.groups[ch.id]
              ? { type: "move_group_card", id: ch.id, graphPos }
              : st.junctions[ch.id]
                ? { type: "move_junction_card", id: ch.id, graphPos }
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
    // Solver-named capacity bindings — the honest red (efficiency grammar).
    const bottlenecks = bottleneckEdges(df);
    const geoms: Record<string, NodeGeom> = {};
    for (const n of nodes) {
      const m = (n as { measured?: { width?: number; height?: number } }).measured;
      geoms[n.id] = {
        x: n.position.x,
        y: n.position.y,
        w: m?.width ?? (n.type === "group" ? 248 : n.type === "junction" ? 84 : 96),
        h: m?.height ?? (n.type === "group" ? 150 : n.type === "junction" ? 84 : 96),
      };
    }
    // Chip footprint from the real text (mono ≈ 6.4px/char at 10px + padding).
    const labelSizes: Record<string, LabelSize> = {};
    for (const e of beltEdges) {
      const d = df?.edges[e.id];
      const text = `${fmtRate(d?.flow ?? 0)}/${fmtRate(beltCapacity(e.tier))} · ${fmtPercent(d?.saturation ?? 0)} MK.${e.tier}`;
      labelSizes[e.id] = { w: text.length * 6.4 + 16, h: 20 };
    }
    // Splitters/mergers route belts to distinct faces like the real building.
    const shapes: Record<string, JunctionShape> = {};
    for (const j of Object.values(plan.junctions)) {
      if (j.factory !== factoryId) continue;
      if (j.kind === "merger") shapes[j.id] = "merger";
      else if (j.kind !== "storage") shapes[j.id] = "splitter";
    }
    const layout = computeEdgeLayout(
      geoms,
      beltEdges.map((e) => ({ id: e.id, source: e.from.id, target: e.to.id })),
      labelSizes,
      shapes,
    );
    // Portal stubs: on a filtered floor, a cross-floor belt runs from its
    // on-floor card to a lift portal instead of dimming into noise. Stubs on
    // the same card face fan out so several lifts stay distinct.
    const portalCounts = new Map<string, number>();
    return beltEdges.map((e) => {
      const d = df?.edges[e.id];
      const floorOfEnd = (end: { kind: string; id: string }) =>
        end.kind === "group"
          ? groupFloor(end.id)
          : end.kind === "junction"
            ? useStore.getState().plan.junctions[end.id]?.floor ?? 0
            : 0;
      const srcFloor = floorOfEnd(e.from);
      const dstFloor = floorOfEnd(e.to);
      const lift = srcFloor !== dstFloor;
      // Trace dim: an edge stays lit only when it links two on-chain nodes.
      const traceDim = !!traceSet && !(traceSet.has(e.from.id) && traceSet.has(e.to.id));
      let dimmed = traceDim || (floorFilter !== "all" && srcFloor !== floorFilter && dstFloor !== floorFilter);
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
            // A cross-floor stub is un-dimmed by the floor filter, but a trace
            // selection still owns it: keep it dim when it's off the traced chain.
            if (!traceDim) dimmed = false;
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
          bottleneck: bottlenecks.has(e.id),
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
  }, [factory, plan.edges, plan.junctions, factoryId, df, selection, isProjected, flowOverlay, settled, nodes, floorFilter, groupFloor, jumpFloor, traceSet]);

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
      const junctionItems = (id: string): string[] => {
        // a junction carries whatever already flows through it; empty = wildcard
        const j = p.junctions[id];
        if (!j) return [];
        const touching = Object.values(p.edges).filter(
          (e) => (e.from.kind === "junction" && e.from.id === id) || (e.to.kind === "junction" && e.to.id === id),
        );
        return [...new Set(touching.map((e) => e.item))];
      };
      const produced = (id: string): string[] | "any" => {
        const g = p.groups[id];
        if (g) return (gd.recipes[g.recipe]?.products ?? []).map(([item]) => item);
        if (p.junctions[id]) {
          const items = junctionItems(id);
          return items.length ? items : "any";
        }
        const port = p.ports[id];
        return port && port.direction === "in" ? [port.item] : [];
      };
      const consumed = (id: string): string[] | "any" => {
        const g = p.groups[id];
        if (g) return (gd.recipes[g.recipe]?.ingredients ?? []).map(([item]) => item);
        if (p.junctions[id]) {
          const items = junctionItems(id);
          return items.length ? items : "any";
        }
        const port = p.ports[id];
        return port && port.direction === "out" ? [port.item] : [];
      };
      if (!conn.source || !conn.target) return;
      const prod = produced(conn.source);
      const cons = consumed(conn.target);
      let item: string | undefined;
      if (prod === "any" && cons === "any") item = undefined; // two blank junctions — nothing to infer
      else if (prod === "any") item = (cons as string[])[0];
      else if (cons === "any") item = (prod as string[])[0];
      else item = (prod as string[]).find((i) => (cons as string[]).includes(i));
      if (!item) return; // no shared item — connection is meaningless, refuse silently
      const endOf = (id: string) =>
        p.groups[id]
          ? { kind: "group" as const, id }
          : p.junctions[id]
            ? { kind: "junction" as const, id }
            : { kind: "port" as const, id };
      void dispatch([{ type: "add_edge", factory: factoryId, from: endOf(conn.source), to: endOf(conn.target), item, tier: 1 }]);
    },
    [dispatch, factoryId],
  );

  // ---- keys: ESC world, ⌫ delete planned, R recipes ----
  const [stripOpen, setStripOpen] = useState(true);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (isEditableTarget(e)) return;
      if (e.key === "Escape") {
        if (addMenu) {
          setAddMenu(null);
          return;
        }
        setCtx(null);
        // A marquee box-selection lives in React Flow, not the store, so clear
        // it here — otherwise Escape would skip past it and eject to the map.
        if (getNodes().some((n) => n.selected)) {
          boxSelRef.current = false;
          setNodes((ns) => ns.map((n) => (n.selected ? { ...n, selected: false } : n)));
          setSelection(null);
        } else if (selection) setSelection(null);
        else setView({ mode: "map" });
      } else if (e.key === "Backspace" || e.key === "Delete") {
        const sel = useStore.getState().selection;
        // Delete a box-selection (which lives in React Flow, so store selection
        // is null) — remove every selected group / junction / port at once.
        const boxed = getNodes().filter((n) => n.selected);
        if (!sel && boxed.length) {
          const st = useStore.getState().plan;
          void dispatch(
            boxed
              .map((n): Command | null =>
                st.groups[n.id]
                  ? { type: "delete_group", id: n.id }
                  : st.junctions[n.id]
                    ? { type: "delete_junction", id: n.id }
                    : st.ports[n.id]
                      ? { type: "delete_port", id: n.id }
                      : null,
              )
              .filter((c): c is Command => c !== null),
          );
          return;
        }
        if (!sel) return;
        const del: Command[] | null =
          sel.kind === "group"
            ? [{ type: "delete_group", id: sel.id }]
            : sel.kind === "edge"
              ? [{ type: "delete_edge", id: sel.id }]
              : sel.kind === "port"
                ? [{ type: "delete_port", id: sel.id }]
                : sel.kind === "junction"
                  ? [{ type: "delete_junction", id: sel.id }]
                  : null;
        if (!del) {
          setSelection(null);
          return;
        }
        // keep the selection when the backend refuses (e.g. ◆ built entities)
        void dispatch(del).then((r) => {
          if (r) setSelection(null);
        });
      } else if (e.key === "r" || e.key === "R") {
        setStripOpen((o) => !o);
      } else if (e.key === "f" || e.key === "F") {
        // leave room for the inspector panel (360px) so framed cards are
        // never hidden under it
        void fitView({
          padding: useStore.getState().selection ? { top: 0.15, bottom: 0.15, left: 0.15, right: 0.32 } : 0.15,
          duration: 200,
        });
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selection, addMenu, dispatch, setSelection, setView, fitView, getNodes]);

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
          onClick={() => setBuildSheet(true)}
          title="BUILD SHEET — a clean, copy-friendly per-factory checklist to build from in-game"
          data-testid="btn-build-sheet"
        >
          BUILD SHEET
        </button>
        <button
          className="btn btn-ghost overlay-chip"
          onClick={() => void dispatch([{ type: "tidy_layout", factory: factoryId }])}
          title="Re-lay every card left→right by flow (inputs → stages → outputs) — one undo step"
          data-testid="btn-tidy"
        >
          TIDY
        </button>
        <button
          className="btn btn-ghost overlay-chip"
          onClick={autoFloor}
          title="Assign floors by production stage (inputs low, assembly high) and stack — one undo step"
          data-testid="btn-auto-floor"
        >
          AUTO-FLOOR
        </button>
        <button
          className="btn btn-ghost overlay-chip"
          onClick={() => void runT2()}
          disabled={t2Busy}
          title="T2 — recipe optimization: alternates diffed into a mini-proposal, never applied directly"
          data-testid="btn-t2"
        >
          {t2Busy ? "OPTIMIZING…" : t2Note ?? "OPTIMIZE (T2)"}
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
        <button
          className="btn btn-ghost"
          data-testid="btn-add-machine"
          title="Add a machine group — same as double-clicking the canvas"
          onClick={() => {
            const rect = flowRef.current!.getBoundingClientRect();
            setAddMenu({
              x: rect.width / 2 - 110,
              y: rect.height / 3,
              flowX: rect.left + rect.width / 2,
              flowY: rect.top + rect.height / 3,
            });
          }}
        >
          + MACHINE
        </button>
        <div style={{ position: "relative" }}>
          <button className="btn btn-ghost" onClick={() => setLogisticMenu((o) => !o)} data-testid="btn-logistic">
            + LOGISTIC
          </button>
          {logisticMenu && (
            <div className="logistic-menu" data-testid="logistic-menu">
              {(
                [
                  ["splitter", "Conveyor Splitter"],
                  ["smart_splitter", "Smart Splitter"],
                  ["programmable_splitter", "Programmable Splitter"],
                  ["merger", "Conveyor Merger"],
                  ["storage", "Storage Container"],
                ] as const
              ).map(([kind, fallback]) => {
                const cls = {
                  splitter: "Build_ConveyorAttachmentSplitter_C",
                  smart_splitter: "Build_ConveyorAttachmentSplitterSmart_C",
                  programmable_splitter: "Build_ConveyorAttachmentSplitterProgrammable_C",
                  merger: "Build_ConveyorAttachmentMerger_C",
                  storage: "Build_StorageContainerMk1_C",
                }[kind];
                const name = useStore.getState().gamedata.buildables?.[cls]?.displayName ?? fallback;
                return (
                  <button
                    key={kind}
                    className="addgroup-item"
                    onClick={() => {
                      const rect = flowRef.current!.getBoundingClientRect();
                      const pos = screenToFlow({ x: rect.left + rect.width / 2, y: rect.top + rect.height / 3 });
                      void dispatch(
                        [
                          {
                            type: "add_junction",
                            factory: factoryId,
                            kind,
                            graphPos: { x: Math.round(pos.x / 16) * 16, y: Math.round(pos.y / 16) * 16 },
                            floor: floorFilter === "all" ? 0 : floorFilter,
                          },
                        ],
                        { select: true },
                      );
                      setLogisticMenu(false);
                    }}
                  >
                    <ItemIcon item={cls} displayName={name} size={20} />
                    <span>{name}</span>
                  </button>
                );
              })}
            </div>
          )}
        </div>
        <button className="btn btn-ghost" onClick={() => setPortMenu("in")}>
          + IN PORT
        </button>
        <button className="btn btn-ghost" onClick={() => setPortMenu("out")}>
          + OUT PORT
        </button>
        <button
          className="btn btn-primary"
          onClick={() => setMakeOpen(true)}
          title="MAKE FROM RESOURCES — pick an item makeable from this factory's inputs and auto-build the chain"
          data-testid="btn-make-from-resources"
        >
          ✦ MAKE
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
        {(plan.factories[factoryId]?.groups ?? []).length === 0 &&
          !Object.values(plan.junctions).some((j) => j.factory === factoryId) &&
          !addMenu && (
          // Empty factory: teach the add gesture — the canvas has no visible
          // affordance for it otherwise. pointer-events: none, so the taught
          // double-click lands on the pane straight through this hint. Groups,
          // junctions, or an open add menu clear it; ports alone don't — the
          // onboarding flow is claim a node (creates the input port) → open
          // the graph → add the first machine, with the hint still teaching.
          <div className="graph-empty-hint mono" data-testid="graph-empty-hint">
            <span className="t-label">DOUBLE-CLICK THE CANVAS TO ADD A MACHINE GROUP</span>
            <span>pick what to make — the machine follows · or use + MACHINE above</span>
          </div>
        )}
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
            setCtx(null);
          }}
          onSelectionStart={() => {
            boxSelRef.current = true;
            setSelection(null);
            setCtx(null);
          }}
          onSelectionEnd={() => {
            boxSelRef.current = false;
          }}
          onNodeClick={narrowOnClick}
          onNodeContextMenu={openNodeCtx}
          onSelectionContextMenu={openSelectionCtx}
          onPaneContextMenu={(e) => {
            e.preventDefault();
            setCtx(null);
          }}
          zoomOnDoubleClick={false}
          snapToGrid
          snapGrid={[16, 16]}
          minZoom={0.25}
          maxZoom={2}
          fitView
          proOptions={{ hideAttribution: true }}
          deleteKeyCode={null}
          // Left-drag on the canvas box-selects the machines it touches; scroll/
          // trackpad pans and pinch/ctrl-scroll zooms, so drag-to-pan giving way
          // to marquee select still leaves the graph fully navigable.
          selectionOnDrag
          panOnDrag={false}
          panOnScroll
          selectionMode={SelectionMode.Partial}
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

      {buildSheet && <BuildSheet factoryId={factoryId} onClose={() => setBuildSheet(false)} />}
      {makeOpen && <MakeFromResources factoryId={factoryId} onClose={() => setMakeOpen(false)} />}
      {ctx && <GraphContextMenu target={ctx} factoryId={factoryId} onClose={() => setCtx(null)} />}
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
