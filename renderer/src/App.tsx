import { useEffect, useRef, useState, type ReactNode } from "react";
import BootScreen, { type BootPhase } from "./shell/BootScreen";
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
import ToastHost from "./shell/ToastHost";
import Dashboard from "./dashboard/Dashboard";
import MobileDashboard from "./mobile/MobileDashboard";
import { useStore, errText } from "./state/store";
import { isEditableTarget } from "./lib/keys";
import "./shell/shell.css";

export default function App() {
  const { mode } = useLayoutMode();
  useAutoZoom(); // shell only: shrink CSS px on low-logical-res displays (4K TV at 300% scaling)
  const [auditOpen, setAuditOpen] = useState(false);
  // MANIFOLD boot overlay lifecycle: "load" while the expanding manifold
  // tracks real hydrate progress, "reveal" during the 0.5s splash → map
  // crossfade (the app frame plays .boot-reveal beneath), "done" once the
  // overlay is gone for good. Never re-enters — hydrate() re-runs (plan
  // switches) patch the live app, only the first boot gets the choreography.
  const [bootPhase, setBootPhase] = useState<BootPhase>("load");
  const ready = useStore((s) => s.ready);
  const error = useStore((s) => s.error);
  const view = useStore((s) => s.view);
  const reviewing = useStore((s) => s.reviewing);
  const reviewingProposal = useStore((s) => (s.reviewing ? s.plan.proposals[s.reviewing] ?? null : null));
  const dashboardOpen = useStore((s) => s.dashboardOpen);
  const emptyPlan = useStore((s) => Object.keys(s.plan.factories).length === 0);
  const auditRequest = useStore((s) => s.auditRequest);
  const planHash = useStore((s) => s.planHash);
  const hydrate = useStore((s) => s.hydrate);
  const initWebllm = useStore((s) => s.initWebllm);
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
    // On-device AI: reconcile the persisted opt-in with live WebGPU support and,
    // if previously enabled, warm the cached weights so NEXT MOVES ranks
    // on-device without a second click. Cheap no-op when never opted in.
    initWebllm();
  }, [hydrate, initWebllm]);

  // Auto-present the resume dashboard ONCE per plan when there's work to resume
  // (a non-empty build queue OR open re-import drift) — else fall straight
  // through to the map (or Onboarding for an empty plan). The `resumeSeen` flag
  // is PERSISTED in viewState (like `onboarded`), so the greeting fires once and
  // never ambushes the restored map on later opens; the H key + StatusBar chip
  // reopen it on demand.
  useEffect(() => {
    if (!ready) return;
    // Phone breakpoint renders the read-only MobileDashboard — never burn the
    // once-per-plan resume greeting (a WRITE) from a glance-only surface.
    if (mode === "phone") return;
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
  }, [ready, mode]);

  // PR 9 openAudit action: the drawer's open flag is local state here, so a
  // store-level request opens it; the drawer itself selects the requested tab
  // and clears the request.
  useEffect(() => {
    if (auditRequest) setAuditOpen(true);
  }, [auditRequest]);

  // PR 3: the SINGLE central per-edit merge. The shared NEXT rank state has one
  // owner; folding the free heuristic list under the model's standing order
  // must happen exactly once per edit regardless of how many feed surfaces are
  // open — mergeOnEdit dedups on planHash internally (and no-ops until a feed
  // has ranked at least once, so an untouched app never derives on every edit).
  useEffect(() => {
    void useStore.getState().mergeOnEdit();
  }, [planHash]);

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
        // H toggles the resume dashboard (home). Inside the factory graph H is
        // the Pan-tool hotkey (Figma hand) which GraphView owns — but only while
        // the graph is the active surface. When the dashboard is layered OVER
        // the graph, H still closes it (as on the map); consume the key so
        // GraphView's Pan handler doesn't also flip the tool underneath. Escape
        // stays the graph's way back to the map, so H=home is one keystroke away.
        const st = useStore.getState();
        if (st.view.mode === "factory" && !st.dashboardOpen) return;
        if (!st.reviewing) {
          st.setDashboardOpen(!st.dashboardOpen);
          if (st.view.mode === "factory") e.stopImmediatePropagation();
        }
      } else if (e.key === "Escape") {
        const st = useStore.getState();
        // The wizard and the review surface own their Escape flows, and a
        // focused text field owns its own Escape semantics (e.g. dismissing a
        // combobox list) — this capture-phase handler must not steal the key
        // from any of them to close the dashboard/advisor underneath.
        if (st.wizard.open || st.reviewing || isEditableTarget(e)) return;
        // The AI-settings popover layers OVER the dashboard, and this capture
        // handler fires before any popover-local listener could — so the
        // popover defers through the store (aiSettingsOpen, the same pattern
        // as wizard/reviewing above) and Escape closes the TOP layer only:
        // popover first, dashboard on the next press. M2: the flag now names
        // the owning header (context) rather than a bare boolean — non-null
        // means some popover is open; closing clears it wholesale.
        if (st.aiSettingsOpen !== null) {
          st.setAiSettingsOpen(null);
          e.preventDefault();
          e.stopImmediatePropagation();
          return;
        }
        // Consume Escape when it dismisses the dashboard: capture-phase + stop
        // so MapView's window Escape handler doesn't ALSO clear the map
        // selection for the same keystroke. Other keys still reach MapView.
        if (st.dashboardOpen) {
          st.setDashboardOpen(false);
          e.preventDefault();
          e.stopImmediatePropagation();
          return;
        }
        // Advisor panel closes on Escape like every other overlay (it opens
        // via A or its edge tab; × was its only way out — a keyboard dead end).
        if (st.advisorOpen) {
          st.setAdvisorOpen(false);
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
  // window must never lose its drag region and min/max/close. At the phone
  // breakpoint (web build — Tauri's minWidth 800 can't get here) the window
  // controls are inert browser chrome, so the card stands alone.
  const screen = (body: ReactNode) => (
    <div className="app-frame" data-layout={mode === "phone" ? "phone" : "overlay"}>
      {mode !== "phone" && <Titlebar overlayMode={false} />}
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

  // Single return so the BootScreen keeps ONE component identity across the
  // ready flip — an early `!ready` return would remount it at a different
  // tree position mid-choreography, resetting the sim (and misreading a slow
  // boot as the fast path).
  // Phone breakpoint (#110): the editing surfaces degrade badly on touch, so
  // the ready app swaps the whole shell for the read-only companion dashboard —
  // empire power, alerts, and the resource ledger at a glance. Resizing back
  // re-enters the full app (same store, only component trees swap). The branch
  // lives INSIDE the single return so BootScreen keeps its one identity.
  return (
    <>
      {ready && mode === "phone" && (
        <>
          <MobileDashboard />
          <ToastHost />
        </>
      )}
      {ready && mode !== "phone" && (
        <div className={`app-frame ${bootPhase === "reveal" ? "boot-reveal" : ""}`} data-layout={mode}>
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
        <ToastHost />
      </main>
          <StatusBar overlayMode={mode === "overlay"} />
        </div>
      )}
      {bootPhase !== "done" && <BootScreen phase={bootPhase} setPhase={setBootPhase} />}
    </>
  );
}
