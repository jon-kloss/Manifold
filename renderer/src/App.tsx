import { useEffect, useState } from "react";
import Titlebar from "./shell/Titlebar";
import StatusBar from "./shell/StatusBar";
import { useLayoutMode } from "./shell/useLayoutMode";
import MapView from "./map/MapView";
import GraphView from "./graph/GraphView";
import AuditDrawer from "./audit/AuditDrawer";
import WizardModal from "./wizard/WizardModal";
import ProposalReview from "./proposal/ProposalReview";
import AdvisorPanel from "./advisor/AdvisorPanel";
import Onboarding from "./shell/Onboarding";
import { useStore } from "./state/store";
import "./shell/shell.css";

export default function App() {
  const { mode, width, height } = useLayoutMode();
  const [auditOpen, setAuditOpen] = useState(false);
  const ready = useStore((s) => s.ready);
  const error = useStore((s) => s.error);
  const view = useStore((s) => s.view);
  const reviewing = useStore((s) => s.reviewing);
  const reviewingProposal = useStore((s) => (s.reviewing ? s.plan.proposals[s.reviewing] ?? null : null));
  const hydrate = useStore((s) => s.hydrate);
  const undo = useStore((s) => s.undo);
  const redo = useStore((s) => s.redo);

  useEffect(() => {
    void hydrate();
  }, [hydrate]);

  // Global keys: ⌘Z / ⌘⇧Z include solve-induced changes by construction.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "z") {
        e.preventDefault();
        void (e.shiftKey ? redo() : undo());
      } else if (e.key === "Tab" && !(e.target instanceof HTMLInputElement) && !(e.target instanceof HTMLSelectElement)) {
        // TAB toggles the audit HUD (mock 1i)
        e.preventDefault();
        setAuditOpen((o) => !o);
      } else if (
        (e.key === "a" || e.key === "A") &&
        !e.metaKey &&
        !e.ctrlKey &&
        !(e.target instanceof HTMLInputElement) &&
        !(e.target instanceof HTMLSelectElement)
      ) {
        const st = useStore.getState();
        st.setAdvisorOpen(!st.advisorOpen);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [undo, redo]);

  if (mode === "refuse") {
    // A1: refuse gracefully, never render broken.
    return (
      <div className="refuse-wrap">
        <div className="refuse-card">
          <h1 className="t-title">FICSIT PLANNER NEEDS AT LEAST 1366×768</h1>
          <div className="mono">
            CURRENT {width}×{height}
          </div>
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="refuse-wrap">
        <div className="refuse-card">
          <h1 className="t-title">BACKEND UNREACHABLE</h1>
          <div className="mono">{error}</div>
        </div>
      </div>
    );
  }

  if (!ready) {
    return (
      <div className="refuse-wrap">
        <div className="mono" style={{ color: "var(--ink-500)" }}>
          HYDRATING…
        </div>
      </div>
    );
  }

  return (
    <div className="app-frame" data-layout={mode}>
      <Titlebar overlayMode={mode === "overlay"} />
      <main className="app-canvas">
        {view.mode === "map" || reviewing ? (
          <MapView />
        ) : (
          <GraphView key={view.factoryId} factoryId={view.factoryId} />
        )}
        {!reviewing && <AuditDrawer open={auditOpen} onToggle={() => setAuditOpen((o) => !o)} />}
        {reviewingProposal && <ProposalReview proposal={reviewingProposal} />}
        {!reviewing && <AdvisorPanel />}
        <WizardModal />
        <Onboarding />
      </main>
      <StatusBar overlayMode={mode === "overlay"} />
    </div>
  );
}
