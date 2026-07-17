// ⌘K search: find item, factory, node… (mock 2a top-left, 280×30).

import { useEffect, useMemo, useRef, useState } from "react";
import { useStore } from "../state/store";
import { itemLabel } from "../lib/format";

interface Hit {
  key: string;
  label: string;
  sub: string;
  pos: { x: number; y: number };
  select: () => void;
}

export default function SearchBox({ onJump }: { onJump: (pos: { x: number; y: number }) => void }) {
  const inputRef = useRef<HTMLInputElement>(null);
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);
  const plan = useStore((s) => s.plan);
  const world = useStore((s) => s.world);
  const gamedata = useStore((s) => s.gamedata);
  const setSelection = useStore((s) => s.setSelection);
  const setMapFilter = useStore((s) => s.setMapFilter);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        inputRef.current?.focus();
        setOpen(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const hits: Hit[] = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return [];
    const out: Hit[] = [];
    for (const f of Object.values(plan.factories)) {
      if (f.name.toLowerCase().includes(q))
        out.push({
          key: `f-${f.id}`,
          label: f.name.toUpperCase(),
          sub: "FACTORY",
          pos: f.position,
          select: () => setSelection({ kind: "factory", id: f.id }),
        });
    }
    for (const n of world.nodes) {
      const item = itemLabel(gamedata.items, n.item);
      if (item.toLowerCase().includes(q) || n.id.includes(q))
        out.push({
          key: `n-${n.id}`,
          label: `${item.toUpperCase()} · ${n.purity.toUpperCase()}`,
          sub: "NODE",
          pos: { x: n.x, y: n.y },
          select: () => setSelection({ kind: "node", id: n.id }),
        });
    }
    return out.slice(0, 8);
  }, [query, plan.factories, world.nodes, gamedata.items, setSelection]);

  // Clearing the box lifts the map filter — never leave the resource field
  // narrowed after the search is gone (also on unmount, e.g. opening a factory).
  const setBoth = (q: string) => {
    setQuery(q);
    setMapFilter(q);
  };
  useEffect(() => () => setMapFilter(""), [setMapFilter]);

  const jump = (h: Hit) => {
    onJump(h.pos);
    h.select();
    setOpen(false);
    setBoth("");
    inputRef.current?.blur();
  };

  return (
    <div className="searchbox">
      <input
        ref={inputRef}
        placeholder="⌘K — find item, factory, node…"
        value={query}
        onChange={(e) => {
          setBoth(e.target.value);
          setOpen(true);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter" && hits[0]) jump(hits[0]);
          if (e.key === "Escape") {
            setBoth("");
            setOpen(false);
            e.currentTarget.blur();
          }
        }}
        onBlur={() => setTimeout(() => setOpen(false), 150)}
      />
      {open && hits.length > 0 && (
        <div className="search-results">
          {hits.map((h) => (
            <button key={h.key} className="search-hit" onMouseDown={() => jump(h)}>
              <span>{h.label}</span>
              <span className="mono search-sub">{h.sub}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
