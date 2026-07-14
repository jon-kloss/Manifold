import { useEffect, useRef, useState, type ReactNode } from "react";
import Titlebar from "./shell/Titlebar";
import StatusBar from "./shell/StatusBar";
import { useLayoutMode } from "./shell/useLayoutMode";
import { useAutoZoom } from "./shell/useAutoZoom";
import MapView from "./map/MapView";
import GraphView from "./graph/GraphView";
import AuditDrawer from "./audit/AuditDrawer";
import WizardModal from "./wizard/WizardModal";
import ProposalReview from "./proposal/ProposalReview";
import AdvisorPanel from "./advisor/AdvisorPanel";
import Onboarding from "./shell/Onboarding";
import Dashboard from "./dashboard/Dashboard";
import { useStore, errText } from "./state/store";
import { isEditableTarget } from "./lib/keys";
import "./shell/shell.css";

export default function App() {
  const { mode } = useLayoutMode();
  useAutoZoom(); // shell only: shrink CSS px on low-logical-res displays (4K TV at 300% scaling)
  const [auditOpen, setAuditOpen] = useState(false);
  const ready = useStore((s) => s.ready);
  const error = useStore((s) => s.error);
  const view = useStore((s) => s.view);
  const reviewing = useStore((s) => s.reviewing);
  const reviewingProposal = useStore((s) => (s.reviewing ? s.plan.proposals[s.reviewing] ?? null : null));
  const dashboardOpen = useStore((s) => s.dashboardOpen);
  const emptyPlan = useStore((s) => Object.keys(s.plan.factories).length === 0);
  const hydrate = useStore((s) => s.hydrate);
  const undo = useStore((s) => s.undo);
  const redo = useStore((s) => s.redo);

  // Hydrate exactly once. StrictMode double-invokes mount effects in dev, and a
  // second hydrate would re-`set` viewState from disk — clobbering the
  // `resumeSeen` flag the auto-present effect persists a beat later, so the
  // dashboard would ambush every later reload. The ref makes it idempotent.
  const hydrated = useRef(false);
  useEffect(() => {
    if (hydrated.current) return;
    hydrated.current = true;
    void hydrate();
  }, [hydrate]);

  // Auto-present the resume dashboard ONCE per plan when there's work to resume
  // (a non-empty build queue OR open re-import drift) — else fall straight
  // through to the map (or Onboarding for an empty plan). The `resumeSeen` flag
  // is PERSISTED in viewState (like `onboarded`), so the greeting fires once and
  // never ambushes the restored map on later opens; the H key + StatusBar chip
  // reopen it on demand.
  useEffect(() => {
    if (!ready) return;
    const st = useStore.getState();
    if (st.viewState.resumeSeen) return;
    const hasDrift = Object.values(st.plan.proposals).some(
      (p) => p.source === "save_reimport" && (p.status === "draft" || p.status === "reviewing"),
    );
    const hasWork = st.derived.buildQueue.length > 0 || hasDrift;
    const empty = Object.keys(st.plan.factories).length === 0;
    // Burn the once-per-plan flag ONLY when we actually present. A build-from-
    // scratch plan opens empty first; spending the flag there (unconditionally)
    // would mean the dashboard never auto-presents once work exists.
    if (hasWork && !st.reviewing && !empty) {
      st.saveViewState({ resumeSeen: true });
      st.setDashboardOpen(true);
    }
  }, [ready]);

  // Backstop: a rejection that escaped every local handler still lands in
  // the status-bar chip instead of dying silently in the console.
  useEffect(() => {
    const onRejection = (e: PromiseRejectionEvent) => {
      console.error(e.reason);
      useStore.getState().reportCmdError(errText(e.reason));
      e.preventDefault();
    };
    window.addEventListener("unhandledrejection", onRejection);
    return () => window.removeEventListener("unhandledrejection", onRejection);
  }, []);

  // Global keys: ⌘Z / ⌘⇧Z include solve-induced changes by construction.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "z") {
        // typing: leave the event alone so native text undo keeps working
        if (isEditableTarget(e)) return;
        e.preventDefault();
        void (e.shiftKey ? redo() : undo());
      } else if (e.key === "Tab" && !isEditableTarget(e)) {
        // TAB toggles the audit HUD (mock 1i)
        e.preventDefault();
        setAuditOpen((o) => !o);
      } else if ((e.key === "a" || e.key === "A") && !e.metaKey && !e.ctrlKey && !isEditableTarget(e)) {
        const st = useStore.getState();
        st.setAdvisorOpen(!st.advisorOpen);
      } else if ((e.key === "h" || e.key === "H") && !e.metaKey && !e.ctrlKey && !isEditableTarget(e)) {
        // H toggles the resume dashboard (home) — but never over a review.
        const st = useStore.getState();
        if (!st.reviewing) st.setDashboardOpen(!st.dashboardOpen);
      } else if (e.key === "Escape") {
        const st = useStore.getState();
        // Consume Escape when it dismisses the dashboard: capture-phase + stop
        // so MapView's window Escape handler doesn't ALSO clear the map
        // selection for the same keystroke. Other keys still reach MapView.
        if (st.dashboardOpen) {
          st.setDashboardOpen(false);
          e.preventDefault();
          e.stopImmediatePropagation();
          return;
        }
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [undo, redo]);

  // Early screens (error / hydrating) still get the titlebar: the frameless
  // window must never lose its drag region and min/max/close.
  const screen = (body: ReactNode) => (
    <div className="app-frame" data-layout="overlay">
      <Titlebar overlayMode={false} />
      <main className="app-canvas">
        <div className="refuse-wrap">{body}</div>
      </main>
    </div>
  );

  if (error) {
    return screen(
      <div className="refuse-card">
        <h1 className="t-title">BACKEND UNREACHABLE</h1>
        <div className="mono">{error}</div>
      </div>,
    );
  }

  if (!ready) {
    return screen(
      <div className="mono" style={{ color: "var(--ink-500)" }}>
        HYDRATING…
      </div>,
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
        {/* Resume overlay: on top of the restored view, never over a review
            or the empty-plan Onboarding (Principle 1 — reveals the map on dismiss). */}
        {dashboardOpen && !reviewing && !emptyPlan && <Dashboard />}
      </main>
      <StatusBar overlayMode={mode === "overlay"} />
    </div>
  );
}
