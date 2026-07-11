// Proposal review (mocks 3a–3c): banner + 470px change list over the dimmed
// map. Partial accept: excluded rows strike through and the consequence
// recomputes LIVE (footer cells + amber warning strip). Accept = one undo
// step creating ◇ planned entities; the built layer is never touched.

import { useCallback, useEffect, useRef, useState } from "react";
import { useStore } from "../state/store";
import { backend } from "../state/backend";
import { fmtPower, fmtRate } from "../lib/format";
import type { Proposal, ProposalConsequence, ProposalItem } from "../state/types";
import "./proposal.css";

const KIND_ORDER = ["create", "modify", "claim", "route_add"] as const;
const KIND_HEADER: Record<string, { label: string; cls: string; glyph: string }> = {
  create: { label: "CREATE", cls: "create", glyph: "+" },
  modify: { label: "MODIFY", cls: "modify", glyph: "Δ" },
  claim: { label: "CLAIM", cls: "claim", glyph: "◉" },
  route_add: { label: "ROUTE", cls: "route", glyph: "⟶" },
};

export default function ProposalReview({ proposal }: { proposal: Proposal }) {
  const planHash = useStore((s) => s.planHash);
  const dispatch = useStore((s) => s.dispatch);
  const setReviewing = useStore((s) => s.setReviewing);
  const setWizard = useStore((s) => s.setWizard);
  const acceptProposal = useStore((s) => s.acceptProposal);
  const gamedata = useStore((s) => s.gamedata);

  const [consequence, setConsequence] = useState<ProposalConsequence | null>(null);
  const [cursor, setCursor] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);

  const stale = proposal.inputHash !== planHash;
  const includedCount = proposal.items.filter((i) => i.included).length;
  const ordered: ProposalItem[] = KIND_ORDER.flatMap((k) => proposal.items.filter((i) => i.kind === k));

  const evalNow = useCallback(() => {
    backend
      .proposalEval(proposal.id)
      .then(setConsequence)
      .catch(() => setConsequence(null));
  }, [proposal.id]);

  useEffect(() => {
    evalNow();
  }, [evalNow, proposal]);

  const toggle = useCallback(
    (item: ProposalItem) => {
      void dispatch([
        { type: "toggle_proposal_item", proposal: proposal.id, item: item.id, included: !item.included },
      ]);
    },
    [dispatch, proposal.id],
  );

  const accept = useCallback(() => {
    if (includedCount === 0) return;
    void acceptProposal(proposal.id);
  }, [acceptProposal, proposal.id, includedCount]);

  const reject = useCallback(() => {
    void dispatch([{ type: "set_proposal_status", id: proposal.id, status: "rejected" }]);
    setReviewing(null);
  }, [dispatch, proposal.id, setReviewing]);

  // keys (3c): ↑↓ walk, SPACE toggle, ⏎ accept, ESC exit (draft kept)
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement) return;
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        setCursor((c) => {
          const n = Math.max(0, Math.min(ordered.length - 1, c + (e.key === "ArrowDown" ? 1 : -1)));
          listRef.current?.querySelectorAll(".prop-row")[n]?.scrollIntoView({ block: "nearest" });
          return n;
        });
      } else if (e.key === " ") {
        e.preventDefault();
        const item = ordered[cursor];
        if (item) toggle(item);
      } else if (e.key === "Enter") {
        e.preventDefault();
        accept();
      } else if (e.key === "Escape") {
        e.stopPropagation();
        setReviewing(null);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [ordered, cursor, toggle, accept, setReviewing]);

  const goalCell = consequence?.goal[0];
  const itemName = (cls: string) => gamedata.items[cls]?.displayName ?? cls;

  return (
    <div className="prop-review" data-testid="proposal-review">
      {/* banner */}
      <header className="prop-banner">
        <span className="prop-stamp mono">PROPOSAL #{proposal.number}</span>
        <span className="t-title">{proposal.title}</span>
        {stale && (
          <span className="prop-stale mono" title="The plan changed since this was solved">
            STALE
          </span>
        )}
        <span className="mono prop-provenance">
          {proposal.provenance} · SNAPSHOT {proposal.snapshotTime.slice(11, 16)} · NOTHING APPLIES UNTIL YOU ACCEPT
        </span>
        <button className="btn btn-ghost" onClick={() => setReviewing(null)} data-testid="btn-exit-review">
          EXIT REVIEW · ESC
        </button>
      </header>

      {/* change list */}
      <aside className="prop-panel">
        <div className="prop-list" ref={listRef}>
          {KIND_ORDER.map((kind) => {
            const items = proposal.items.filter((i) => i.kind === kind);
            if (items.length === 0) return null;
            const head = KIND_HEADER[kind];
            return (
              <section key={kind}>
                <h3 className={`prop-group-head t-label ${head.cls}`}>
                  {head.label} <span className="mono">{items.length}</span>
                </h3>
                {items.map((item) => {
                  const idx = ordered.indexOf(item);
                  return (
                    <div
                      key={item.id}
                      className={`prop-row ${item.included ? "" : "excluded"} ${idx === cursor ? "cursor" : ""}`}
                      onClick={() => setCursor(idx)}
                      data-testid="proposal-item"
                    >
                      <input
                        type="checkbox"
                        checked={item.included}
                        onChange={() => toggle(item)}
                        onClick={(e) => e.stopPropagation()}
                      />
                      <span className={`prop-glyph mono ${head.cls}`}>{head.glyph}</span>
                      <span className="prop-row-main">
                        <span className="prop-row-label">{item.label}</span>
                        <span className="prop-row-detail">{item.detail}</span>
                      </span>
                      <span className="mono prop-row-impact">{item.impact}</span>
                    </div>
                  );
                })}
              </section>
            );
          })}
        </div>

        {consequence && consequence.warnings.length > 0 && (
          <div className="prop-warnings" data-testid="proposal-warnings">
            ⚠ {consequence.warnings.length} warning{consequence.warnings.length > 1 ? "s" : ""} from exclusions —{" "}
            {consequence.warnings[0]}
            {consequence.goalMet ? " Goal still met." : " Goal NOT met."}
          </div>
        )}

        {/* impact footer */}
        <footer className="prop-footer">
          <div className="prop-impact-grid mono">
            <div className={`prop-cell ${consequence && !consequence.goalMet ? "crit" : ""}`}>
              <span className="prop-cell-label">GOAL CHECK</span>
              {goalCell ? (
                <span data-testid="goal-check">
                  {fmtRate(goalCell.achieved)}/{fmtRate(goalCell.requested)} {consequence?.goalMet ? "✓" : "✗"}{" "}
                  {itemName(goalCell.item)}
                </span>
              ) : (
                <span>—</span>
              )}
            </div>
            <div className="prop-cell">
              <span className="prop-cell-label">Δ POWER</span>
              <span>
                +{fmtPower(consequence?.deltaPowerMw ?? 0)} draw
                {(consequence?.deltaGenerationMw ?? 0) > 0 && ` · +${fmtPower(consequence!.deltaGenerationMw)} gen`}
              </span>
            </div>
            <div className="prop-cell">
              <span className="prop-cell-label">MACHINES</span>
              <span>+{consequence?.machines ?? 0}</span>
            </div>
          </div>
          <div className="prop-actions">
            <button
              className="btn btn-primary"
              onClick={accept}
              disabled={includedCount === 0}
              data-testid="btn-accept-proposal"
            >
              ACCEPT {includedCount} AS PLANNED ⏎
            </button>
            <button
              className="btn btn-ghost"
              onClick={() => {
                setReviewing(null);
                setWizard({
                  open: true,
                  prefill: proposal.goal[0] ? { item: proposal.goal[0][0], rate: proposal.goal[0][1] } : undefined,
                });
              }}
            >
              RE-SOLVE…
            </button>
            <button className="btn btn-ghost prop-reject" onClick={reject}>
              REJECT
            </button>
          </div>
          <div className="prop-microcopy">
            Accepting creates ◇ planned entities only — one undo step. The built layer is never touched.
          </div>
        </footer>
      </aside>
    </div>
  );
}
