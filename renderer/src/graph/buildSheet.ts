// BUILD SHEET composition (read-only): fold EXISTING derived + plan state for
// one factory into a clean, ordered, copy/print-friendly spec — machines &
// clocks, inputs + their honest source, outputs, the routes touching the
// factory (belt/pipe/rail tiers + item + rate + length), and total power. No
// solver math lives here; every number is read from the derived projection.

import { fmtClock, fmtKm, fmtPower, fmtRate } from "../lib/format";
import { prettyClass } from "../lib/format";
import { effClock, effCount } from "../state/types";
import type { Derived, GameData, Id, Plan, RouteKind, World } from "../state/types";

const STATUS_GLYPH = { planned: "◇", under_construction: "◈", built: "◆" } as const;

export interface SheetMachine {
  count: number;
  machine: string;
  clock: string;
  recipe: string;
}
export interface SheetPort {
  item: string;
  rate: number;
  /** honest boundary-port source grammar (FROM ROUTE / FROM NODE CLAIM /
   *  UNROUTED — SUPPLY ASSUMED / TO ROUTE / TO WORLD) */
  source: string;
}
export interface SheetRoute {
  item: string;
  dir: "out" | "in";
  other: string;
  tier: string;
  rate: number;
  /** 3D route length in meters (0 when unknown/flat) */
  lengthM: number;
  segments: number;
}
export interface BuildSheetData {
  name: string;
  region: string;
  statusGlyph: string;
  statusText: string;
  powerMw: number;
  machines: SheetMachine[];
  inputs: SheetPort[];
  outputs: SheetPort[];
  routes: SheetRoute[];
}

/** Human tier/mode label for a route (mirrors the map chip grammar). */
function tierLabel(kind: RouteKind): string {
  switch (kind.kind) {
    case "belt":
      return `Belt Mk.${kind.tier}`;
    case "pipe":
      return `Pipe Mk.${kind.tier}`;
    case "rail":
      return "Rail";
    case "truck":
      return "Truck";
    case "drone":
      return "Drone";
    case "power":
      return "Power";
  }
}

const itemName = (gamedata: GameData, cls: string) =>
  gamedata.items[cls]?.displayName ?? prettyClass(cls);

/** Compose the read-only sheet for one factory from store + derived state. */
export function composeBuildSheet(
  factoryId: Id,
  plan: Plan,
  derived: Derived,
  gamedata: GameData,
  world: World,
): BuildSheetData | null {
  const factory = plan.factories[factoryId];
  if (!factory) return null;
  const df = derived.factories[factoryId];

  const machines: SheetMachine[] = factory.groups
    .map((gid) => plan.groups[gid])
    .filter(Boolean)
    .map((g) => {
      const recipe = gamedata.recipes[g.recipe];
      return {
        count: effCount(g),
        machine: gamedata.machines[g.machine]?.displayName ?? g.machine,
        clock: fmtClock(effClock(g)),
        recipe: recipe?.displayName ?? g.recipe,
      };
    });

  const ports = factory.ports.map((pid) => plan.ports[pid]).filter(Boolean);

  const inputs: SheetPort[] = ports
    .filter((p) => p.direction === "in")
    .map((p) => {
      let source: string;
      if (p.boundRoute) source = "FROM ROUTE";
      else {
        const claimed = Object.values(plan.nodeClaims).some(
          (c) =>
            c.factory === factoryId &&
            world.nodes.find((n) => n.id === c.node)?.item === p.item,
        );
        source = claimed ? "FROM NODE CLAIM" : "UNROUTED — SUPPLY ASSUMED";
      }
      return { item: itemName(gamedata, p.item), rate: df?.ports[p.id] ?? p.rate, source };
    });

  const outputs: SheetPort[] = ports
    .filter((p) => p.direction === "out")
    .map((p) => ({
      item: itemName(gamedata, p.item),
      rate: df?.ports[p.id] ?? p.rate,
      source: p.boundRoute ? "TO ROUTE" : "TO WORLD",
    }));

  // Routes touching this factory: cargo routes whose endpoints (port ids) sit
  // on a port this factory owns. Direction from the owned endpoint (from = out).
  const routes: SheetRoute[] = [];
  for (const r of Object.values(plan.routes)) {
    if (r.kind.kind === "power") continue;
    const fromPort = plan.ports[r.endpoints[0]];
    const toPort = plan.ports[r.endpoints[1]];
    const ownsFrom = fromPort?.factory === factoryId;
    const ownsTo = toPort?.factory === factoryId;
    if (!ownsFrom && !ownsTo) continue;
    const dr = derived.routes[r.id];
    const cls = r.manifest[0]?.[0] ?? dr?.item ?? "";
    const otherFactory = ownsFrom ? toPort?.factory : fromPort?.factory;
    const other = (otherFactory && plan.factories[otherFactory]?.name) || "WORLD";
    routes.push({
      item: itemName(gamedata, cls),
      dir: ownsFrom ? "out" : "in",
      other: other.toUpperCase(),
      tier: tierLabel(r.kind),
      rate: dr?.flow ?? 0,
      lengthM: dr?.lengthM ?? 0,
      segments: Math.max(0, r.path.length - 1),
    });
  }
  routes.sort((a, b) => (a.dir === b.dir ? a.item.localeCompare(b.item) : a.dir === "out" ? -1 : 1));

  return {
    name: factory.name,
    region: factory.region,
    statusGlyph: STATUS_GLYPH[factory.status],
    statusText: factory.status.replace("_", " ").toUpperCase(),
    powerMw: df?.totalPowerMw ?? 0,
    machines,
    inputs,
    outputs,
    routes,
  };
}

/** A faithful plain-text / markdown rendering of the sheet — the COPY payload
 *  and the same content the panel shows. Numbers go through the shared format
 *  helpers so a copy never leaks a 10-digit float. */
export function sheetToText(s: BuildSheetData): string {
  const L: string[] = [];
  L.push(`BUILD SHEET — ${s.name.toUpperCase()}`);
  L.push(`${s.region.toUpperCase()} · ${s.statusGlyph} ${s.statusText} · ${fmtPower(s.powerMw)}`);
  L.push("");

  L.push("MACHINES");
  if (s.machines.length === 0) L.push("- (none)");
  for (const m of s.machines) L.push(`- ${m.count}× ${m.machine} @ ${m.clock} — ${m.recipe}`);
  L.push("");

  L.push("INPUTS");
  if (s.inputs.length === 0) L.push("- (none)");
  for (const p of s.inputs) L.push(`- ${p.item} — ${fmtRate(p.rate)}/min · ${p.source}`);
  L.push("");

  L.push("OUTPUTS");
  if (s.outputs.length === 0) L.push("- (none)");
  for (const p of s.outputs) L.push(`- ${p.item} — ${fmtRate(p.rate)}/min · ${p.source}`);
  L.push("");

  L.push("ROUTES");
  if (s.routes.length === 0) L.push("- (none)");
  for (const r of s.routes) {
    const arrow = r.dir === "out" ? `${r.item} → ${r.other}` : `${r.item} ← ${r.other}`;
    const len = r.lengthM > 0 ? ` · ${fmtKm(r.lengthM)}` : "";
    L.push(`- ${arrow} · ${r.tier} · ${fmtRate(r.rate)}/min${len}`);
  }
  L.push("");

  L.push("POWER");
  L.push(`- ${fmtPower(s.powerMw)} at planned clocks`);

  return L.join("\n");
}
