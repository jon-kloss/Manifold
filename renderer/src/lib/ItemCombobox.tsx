// Searchable item picker: an input that filters the catalog as you type, with
// arrow-key + Enter selection. Replaces raw <select> pickers that stop scaling
// past the fixture catalog (a real Docs.json carries hundreds of items —
// finding "Reinforced Iron Plate" by scrolling a native list is the exact
// friction this app exists to remove). Closed, the input reads the chosen
// item's display name beside its chip; focus clears it into query mode.

import { useEffect, useId, useMemo, useRef, useState } from "react";
import type { GameItem } from "../state/types";
import { itemLabel } from "./format";
import ItemIcon from "./ItemIcon";

export default function ItemCombobox({
  items,
  value,
  onChange,
  testid,
}: {
  items: GameItem[];
  value: string;
  onChange: (className: string) => void;
  testid?: string;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [highlight, setHighlight] = useState(0);
  const wrapRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const listId = useId();

  const selected = items.find((i) => i.className === value);
  // `value` can name an item outside the offered list (e.g. a raw-ore deficit
  // prefill — ores are legitimate goals, just not in the craftable catalog).
  // itemLabel gives a known resource name ("Bauxite") or a humanised class,
  // never a raw Desc_..._C or a blank input that hides what a solve targets.
  const display = selected?.displayName ?? itemLabel({}, value);
  const matches = useMemo(() => {
    const q = query.trim().toLowerCase();
    // Prefix matches outrank substring matches ("iron plate" offers Iron Plate
    // before Reinforced Iron Plate), then name order for determinism.
    return items
      .filter((i) => !q || i.displayName.toLowerCase().includes(q))
      .sort((a, b) => {
        const ap = a.displayName.toLowerCase().startsWith(q) ? 0 : 1;
        const bp = b.displayName.toLowerCase().startsWith(q) ? 0 : 1;
        return ap - bp || a.displayName.localeCompare(b.displayName);
      });
    // No cap: a real Docs.json carries hundreds of items and the whole point is
    // to SCROLL the bounded, scrollable list to find one you can't name. (A
    // .slice here left the list too short to ever overflow, so it never
    // scrolled.)
  }, [items, query]);

  // Keep the arrow-key highlight in view now that the list can be long — a
  // bare highlight state would walk off the bottom of the scroll box.
  useEffect(() => {
    if (!open) return;
    listRef.current?.querySelector<HTMLElement>(".item-combo-option.hl")?.scrollIntoView({ block: "nearest" });
  }, [highlight, open]);

  const pick = (cls: string) => {
    onChange(cls);
    setOpen(false);
    setQuery("");
  };

  return (
    <div
      className="item-combo"
      ref={wrapRef}
      onBlur={(e) => {
        // Close only when focus leaves the whole combobox (input → option
        // clicks keep focus inside the wrapper).
        if (!wrapRef.current?.contains(e.relatedTarget as Node)) {
          setOpen(false);
          setQuery("");
        }
      }}
    >
      <ItemIcon item={value} displayName={selected?.displayName} size={20} />
      <input
        className="mono item-combo-input"
        role="combobox"
        aria-expanded={open}
        aria-controls={listId}
        aria-autocomplete="list"
        aria-activedescendant={open && matches[highlight] ? `${listId}-${highlight}` : undefined}
        value={open ? query : display}
        placeholder={display || "search items…"}
        data-testid={testid}
        onFocus={() => {
          setOpen(true);
          setQuery("");
          setHighlight(0);
        }}
        onChange={(e) => {
          setQuery(e.target.value);
          setHighlight(0);
          setOpen(true);
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape" && open) {
            // Consume: dismiss the list, not the surrounding modal/overlay.
            e.stopPropagation();
            setOpen(false);
            setQuery("");
            (e.target as HTMLInputElement).blur();
          } else if (e.key === "ArrowDown") {
            e.preventDefault();
            setHighlight((h) => Math.min(h + 1, matches.length - 1));
          } else if (e.key === "ArrowUp") {
            e.preventDefault();
            setHighlight((h) => Math.max(h - 1, 0));
          } else if (e.key === "Enter" && open && matches[highlight]) {
            // Consume: this Enter picks an option — it must never double as
            // the surrounding modal's submit key.
            e.preventDefault();
            e.stopPropagation();
            pick(matches[highlight].className);
            (e.target as HTMLInputElement).blur();
          }
        }}
      />
      {open && (
        <div className="item-combo-list" role="listbox" id={listId} ref={listRef}>
          {matches.length === 0 && <div className="item-combo-empty mono">no items match</div>}
          {matches.map((i, idx) => (
            <button
              key={i.className}
              id={`${listId}-${idx}`}
              type="button"
              role="option"
              aria-selected={idx === highlight}
              className={`item-combo-option ${idx === highlight ? "hl" : ""}`}
              data-testid={testid ? `${testid}-option` : undefined}
              onMouseEnter={() => setHighlight(idx)}
              onMouseDown={(e) => e.preventDefault() /* keep input focus until pick */}
              onClick={() => pick(i.className)}
            >
              <ItemIcon item={i.className} displayName={i.displayName} size={20} />
              <span>{i.displayName}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
