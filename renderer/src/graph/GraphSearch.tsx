// Context-aware header search, factory side (#117): typing live-highlights the
// matching machines/items in the open graph by dimming everything else — the
// same idiom as the map's node filter. No dropdown: the canvas IS the result
// list. ⌘K focuses (matching the map search), Escape clears and blurs.

import { useEffect, useRef } from "react";
import { useStore } from "../state/store";

export default function GraphSearch() {
  const inputRef = useRef<HTMLInputElement>(null);
  const graphFilter = useStore((s) => s.graphFilter);
  const setGraphFilter = useStore((s) => s.setGraphFilter);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        inputRef.current?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Leaving the graph clears the filter — a filter that silently survived
  // into the next factory would dim its canvas with no visible cause.
  useEffect(() => () => useStore.getState().setGraphFilter(""), []);

  return (
    <div className="searchbox" data-testid="graph-search">
      <input
        ref={inputRef}
        value={graphFilter}
        placeholder="⌘K — find machine, item…"
        onChange={(e) => setGraphFilter(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            setGraphFilter("");
            (e.target as HTMLInputElement).blur();
            e.stopPropagation(); // don't ALSO eject the graph to the map
          }
        }}
      />
    </div>
  );
}
