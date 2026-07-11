// Collapsible legend (mock 2a, bottom-left, 230px): status glyphs, node states.

import { useState } from "react";

export default function Legend() {
  const [open, setOpen] = useState(true);
  return (
    <div className="map-legend">
      <button className="t-label legend-toggle" onClick={() => setOpen(!open)}>
        LEGEND {open ? "▾" : "▸"}
      </button>
      {open && (
        <div className="legend-body">
          <div className="legend-row">
            <span className="status-planned">◇</span> Planned
            <span className="status-under_construction">◈</span> U/C
            <span className="status-built">◆</span> Built
          </div>
          <div className="legend-row">
            <span className="legend-node pure" /> Pure
            <span className="legend-node normal" /> Normal
            <span className="legend-node impure" /> Impure
          </div>
          <div className="legend-row">
            <span className="legend-node claimed" /> Claimed
            <span className="legend-node conflict" /> Conflict
          </div>
          <div className="legend-row">
            <span className="legend-tether" /> Claim tether (node → factory)
          </div>
          <div className="legend-row">
            <span className="legend-load" /> &lt;70
            <span className="legend-load warn" /> 70–95
            <span className="legend-load crit" /> ≥95
          </div>
        </div>
      )}
    </div>
  );
}
