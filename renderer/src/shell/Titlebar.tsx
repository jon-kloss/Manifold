// Custom titlebar (36px): logo square, app name, breadcrumb, save-sync chip,
// solver status, window controls. Frameless in Tauri; controls hidden in bridge mode.

import { useStore, solveChip } from "../state/store";
import "./shell.css";

const isTauri = "__TAURI_INTERNALS__" in window;

async function windowAction(action: "minimize" | "toggleMaximize" | "close") {
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  const w = getCurrentWindow();
  if (action === "minimize") await w.minimize();
  else if (action === "toggleMaximize") await w.toggleMaximize();
  else await w.close();
}

export default function Titlebar({ overlayMode }: { overlayMode: boolean }) {
  const view = useStore((s) => s.view);
  const plan = useStore((s) => s.plan);
  const derived = useStore((s) => s.derived);
  const setView = useStore((s) => s.setView);

  const factory = view.mode === "factory" ? plan.factories[view.factoryId] : null;
  const chip = solveChip(factory ? derived.factories[factory.id] : undefined);

  return (
    <header className="titlebar" data-tauri-drag-region>
      <div className="titlebar-logo" aria-hidden>
        ◆
      </div>
      {/* #117: no wordmark — the user knows what the tool is. The crumb stays
          (WORLD MAP is the way home from a factory), search sits CENTERED in
          the bar and is context-aware (map view portals the node/factory
          search here; the factory graph portals its machine/item search), and
          the save/load DATA menu docks in the right corner. */}
      <div className="titlebar-slot titlebar-slot-search" id="titlebar-search-slot" />
      <nav className={`titlebar-crumb mono ${overlayMode ? "truncate" : ""}`}>
        <button className="crumb-link" onClick={() => setView({ mode: "map" })}>
          WORLD MAP
        </button>
        {factory && (
          <>
            <span className="crumb-sep">/</span>
            <span className="crumb-here">{factory.name}</span>
          </>
        )}
      </nav>
      <div className="titlebar-right">
        {/* save/load corner slot — filled by MapView's DATA menu portal */}
        <div className="titlebar-slot" id="titlebar-data-slot" />
        <span className="chip" title="Every commit writes the plan file — there is no unsaved state.">
          SAVED ✓
        </span>
        {factory && (
          <span className={`chip ${chip.over ? "warn" : ""}`} data-testid="solve-chip">
            {chip.text}
          </span>
        )}
        {isTauri && (
          <div className="win-controls">
            <button onClick={() => windowAction("minimize")} aria-label="Minimize">
              –
            </button>
            <button onClick={() => windowAction("toggleMaximize")} aria-label="Maximize">
              ▢
            </button>
            <button className="win-close" onClick={() => windowAction("close")} aria-label="Close">
              ×
            </button>
          </div>
        )}
      </div>
    </header>
  );
}
