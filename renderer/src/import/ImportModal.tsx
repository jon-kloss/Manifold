// Save import (SDD §8, onboarding step-3 grammar): .sav → worker parse →
// preview table with honest counts + quarantine → IMPORT AS BUILT. First
// import writes the ◆ Built layer (one undo step); re-imports never write —
// drift arrives as a SaveReimport proposal reviewed like any other.

import { useCallback, useRef, useState } from "react";
import { useStore } from "../state/store";
import { backend } from "../state/backend";
import type { ImportSnapshot } from "../state/types";

type Phase =
  | { step: "parsing"; name: string }
  | { step: "preview"; snapshot: ImportSnapshot }
  | { step: "importing" }
  | { step: "error"; message: string }
  | { step: "done"; message: string };

export default function ImportModal({ file, onClose }: { file: File; onClose: () => void }) {
  const hydrate = useStore((s) => s.hydrate);
  const setReviewing = useStore((s) => s.setReviewing);
  const [phase, setPhase] = useState<Phase | null>(null);
  const started = useRef(false);

  // Snapshot at mount: a FIRST import flips the plan to built mid-flow, and a
  // live read would relabel the header "RE-IMPORT SAVE" while its own done
  // message still reads "imported as ◆ BUILT".
  const [hasBuilt] = useState(() =>
    Object.values(useStore.getState().plan.factories).some((f) => f.status === "built"),
  );

  const start = useCallback(async () => {
    setPhase({ step: "parsing", name: file.name });
    const bytes = await file.arrayBuffer();
    const worker = new Worker(new URL("./parseWorker.ts", import.meta.url), { type: "module" });
    worker.onmessage = (e: MessageEvent<{ snapshot?: ImportSnapshot; error?: string }>) => {
      worker.terminate();
      if (e.data.error || !e.data.snapshot) {
        // no dead ends: parse failure degrades to manual entry
        setPhase({ step: "error", message: e.data.error ?? "empty parse" });
        return;
      }
      setPhase({ step: "preview", snapshot: e.data.snapshot });
    };
    worker.postMessage({ name: file.name, bytes }, [bytes]);
  }, [file]);

  if (!started.current) {
    started.current = true;
    void start();
  }

  const runImport = async (snapshot: ImportSnapshot) => {
    setPhase({ step: "importing" });
    try {
      const outcome = await backend.importRun(snapshot);
      await hydrate(); // the built layer landed backend-side; re-project
      if (outcome.outcome === "imported") {
        setPhase({
          step: "done",
          message: `${outcome.factories} factories · ${outcome.machines} machines imported as ◆ BUILT${
            outcome.quarantined > 0 ? ` · ${outcome.quarantined} modded objects quarantined` : ""
          }`,
        });
      } else if (outcome.outcome === "drift") {
        onClose();
        setReviewing(outcome.proposal);
      } else {
        setPhase({ step: "done", message: "BUILT LAYER IN SYNC — no drift since this save." });
      }
    } catch (err) {
      setPhase({ step: "error", message: String(err) });
    }
  };

  const belts = (s: ImportSnapshot) => Object.values(s.belts ?? {}).reduce((a, b) => a + b, 0);
  const quarantined = (s: ImportSnapshot) => Object.values(s.quarantined ?? {}).reduce((a, b) => a + b, 0);

  return (
    <div className="wizard-scrim" data-testid="import-modal">
      <div className="wizard-modal" style={{ width: 720 }}>
        <header className="wizard-head">
          <span className="wizard-stamp mono">IMPORT</span>
          <span className="t-title">{hasBuilt ? "RE-IMPORT SAVE" : "IMPORT SAVE AS BUILT"}</span>
          <button className="drawer-close" onClick={onClose} aria-label="Close" style={{ marginLeft: "auto" }}>
            ×
          </button>
        </header>
        <div className="wizard-body import-body">
          {phase?.step === "parsing" && (
            <div className="mono" style={{ color: "var(--ink-500)" }}>
              PARSING {phase.name}… (community-reverse-engineered format, in a worker)
            </div>
          )}

          {phase?.step === "preview" && (
            <>
              <div className="import-grid mono" data-testid="import-preview">
                <span>MACHINES</span>
                <span>{phase.snapshot.machines.length}</span>
                <span>EXTRACTORS</span>
                <span>{phase.snapshot.extractors?.length ?? 0}</span>
                <span>BELTS</span>
                <span>{belts(phase.snapshot)}</span>
                <span>RAIL SEGMENTS</span>
                <span>{phase.snapshot.rails ?? 0}</span>
                <span>POWER LINES</span>
                <span>{phase.snapshot.powerLines ?? 0}</span>
                <span>TRAINS</span>
                <span>
                  {phase.snapshot.locomotives ?? 0} LOCO + {phase.snapshot.wagons ?? 0} WAGON ·{" "}
                  {phase.snapshot.trainStations ?? 0} STATIONS
                </span>
                <span>UNRECOGNIZED</span>
                <span>{quarantined(phase.snapshot)} → ignored</span>
              </div>
              {quarantined(phase.snapshot) > 0 && (
                <details className="import-quarantine mono">
                  <summary>VIEW UNRECOGNIZED CLASSES</summary>
                  {Object.entries(phase.snapshot.quarantined ?? {})
                    .sort((a, b) => b[1] - a[1])
                    .slice(0, 12)
                    .map(([cls, n]) => (
                      <div key={cls} className="import-quarantine-row">
                        <span>{cls}</span>
                        <span>×{n}</span>
                      </div>
                    ))}
                  {Object.keys(phase.snapshot.quarantined ?? {}).length > 12 && (
                    <div className="import-quarantine-row">
                      <span>… {Object.keys(phase.snapshot.quarantined ?? {}).length - 12} more classes</span>
                    </div>
                  )}
                </details>
              )}
              <div className="wizard-infeasible" style={{ borderColor: "var(--flow-warn)" }}>
                <span className="wizard-foot-note" style={{ color: "var(--flow-warn)" }}>
                  The save format is community-reverse-engineered. Everything imports as ◆ BUILT
                  {hasBuilt
                    ? " — this re-import never writes: differences arrive as a reviewable drift proposal."
                    : " — your plan is never touched; future re-imports diff against built."}
                </span>
              </div>
              <footer className="wizard-foot">
                <button
                  className="btn btn-primary"
                  onClick={() => void runImport(phase.snapshot)}
                  data-testid="btn-import-run"
                >
                  {hasBuilt ? "DIFF AGAINST BUILT" : "IMPORT AS BUILT"}
                </button>
                <button className="btn btn-ghost" onClick={onClose}>
                  SKIP — MANUAL ENTRY
                </button>
              </footer>
            </>
          )}

          {phase?.step === "importing" && (
            <div className="mono" style={{ color: "var(--ink-500)" }}>
              CLUSTERING MACHINES INTO FACTORIES…
            </div>
          )}

          {phase?.step === "done" && (
            <>
              <div className="mono" data-testid="import-done">
                {phase.message}
              </div>
              <footer className="wizard-foot">
                <button className="btn btn-primary" onClick={onClose}>
                  DONE
                </button>
              </footer>
            </>
          )}

          {phase?.step === "error" && (
            <>
              <div className="wizard-infeasible">
                <span className="t-label" style={{ color: "var(--flow-warn)" }}>
                  PARSE FAILED — SKIP: EVERYTHING WORKS WITH MANUAL ENTRY
                </span>
                <span className="wizard-foot-note">{phase.message.slice(0, 300)}</span>
              </div>
              <footer className="wizard-foot">
                <button className="btn btn-ghost" onClick={onClose}>
                  CLOSE
                </button>
              </footer>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
