// Ambient advisor (mocks 2c feed / 6a chat): a right-edge tab whose loudest
// voice is a badge count. Feed cards carry SAW/RULE provenance and route every
// action through existing review surfaces — the advisor never edits the plan.

import { useCallback, useEffect, useRef, useState } from "react";
import { useStore } from "../state/store";
import { backend } from "../state/backend";
import { fmtRate } from "../lib/format";
import type { AdvisorCard, ChatReply, ChatScope, ContextSnapshot } from "../state/types";
import "./advisor.css";

const SEVERITY_CHIP: Record<string, { label: string; cls: string }> = {
  conflict: { label: "⚠ CONFLICT", cls: "sev-conflict" },
  trend: { label: "▲ TREND", cls: "sev-trend" },
  tip: { label: "● TIP", cls: "sev-tip" },
};

export default function AdvisorPanel() {
  const advisor = useStore((s) => s.advisor);
  const open = useStore((s) => s.advisorOpen);
  const setOpen = useStore((s) => s.setAdvisorOpen);
  const [tab, setTab] = useState<"feed" | "chat">("feed");

  const activeCards = advisor.cards.filter((c) => !c.dismissed);

  if (!open) {
    return (
      <button className="advisor-tab-edge" onClick={() => setOpen(true)} data-testid="advisor-tab" title="Advisor (A)">
        <span className="advisor-tab-label">ADVISOR</span>
        {activeCards.length > 0 && (
          <span className="audit-badge" data-testid="advisor-badge">
            {activeCards.length}
          </span>
        )}
      </button>
    );
  }

  return (
    <aside className="advisor-panel" data-testid="advisor-panel">
      <header className="advisor-head">
        <span className="t-title" style={{ fontSize: 14 }}>
          ADVISOR
        </span>
        <button
          className={`chip ${advisor.paused ? "" : "ambient-on"}`}
          onClick={() => void backend.advisorPause(!advisor.paused).then(useStore.getState().setAdvisor)}
          data-testid="advisor-pause"
          title="Pause silences the ambient rules"
        >
          {advisor.paused ? "◦ PAUSED" : "AMBIENT · ON"}
        </button>
        {advisor.aiStatus === "offline" && (
          <span className="chip advisor-offline" title="No model key — local heuristics keep the feed alive">
            AI OFFLINE
          </span>
        )}
        <button className="drawer-close" onClick={() => setOpen(false)} aria-label="Close">
          ×
        </button>
      </header>
      <div className="advisor-tabs">
        <button className={`audit-tab t-label ${tab === "feed" ? "active" : ""}`} onClick={() => setTab("feed")}>
          FEED
          {activeCards.length > 0 && <span className="audit-badge">{activeCards.length}</span>}
        </button>
        <button className={`audit-tab t-label ${tab === "chat" ? "active" : ""}`} onClick={() => setTab("chat")}>
          CHAT
        </button>
        <span className="mono advisor-budget">
          MODEL CALLS {advisor.callsThisHour}/{advisor.callBudget}·H
        </span>
      </div>
      {tab === "feed" ? <Feed cards={activeCards} muted={advisor.muted} /> : <Chat />}
      <footer className="advisor-foot mono">
        The advisor never edits your plan — every suggestion becomes a proposal you review.
      </footer>
    </aside>
  );
}

function Feed({ cards, muted }: { cards: AdvisorCard[]; muted: string[] }) {
  const setAdvisor = useStore((s) => s.setAdvisor);
  const setWizard = useStore((s) => s.setWizard);
  const setSelection = useStore((s) => s.setSelection);
  const setView = useStore((s) => s.setView);
  const setReviewing = useStore((s) => s.setReviewing);
  const setAdvisorOpen = useStore((s) => s.setAdvisorOpen);
  const gamedata = useStore((s) => s.gamedata);

  const act = (card: AdvisorCard) => {
    const cta = card.cta;
    if (!cta) return;
    if (cta.kind === "planProduction") {
      setAdvisorOpen(false);
      setWizard({ open: true, prefill: { item: cta.item, rate: cta.rate } });
    } else if (cta.kind === "trace") {
      setView({ mode: "map" });
      setSelection({ kind: cta.selection as "node" | "route", id: cta.id });
    } else if (cta.kind === "review") {
      setAdvisorOpen(false);
      setReviewing(cta.proposal);
    }
  };

  return (
    <div className="advisor-body">
      {cards.length === 0 && (
        <div className="drawer-empty" style={{ padding: "12px 2px" }}>
          Silence is a feature — nothing needs your attention.
        </div>
      )}
      {cards.map((c) => {
        const sev = SEVERITY_CHIP[c.severity];
        return (
          <article className="advisor-card" key={c.id} data-testid="advisor-card">
            <div className="advisor-card-head">
              <span className={`advisor-sev mono ${sev.cls}`}>{sev.label}</span>
              <span className="advisor-card-title">{c.title}</span>
            </div>
            <p className="advisor-card-body">{c.body}</p>
            <div className="advisor-card-actions">
              {c.cta?.kind === "planProduction" && (
                <button className="chip warn" onClick={() => act(c)} data-testid="card-cta">
                  CREATE PROPOSAL — {(gamedata.items[c.cta.item]?.displayName ?? c.cta.item).toUpperCase()}{" "}
                  {fmtRate(c.cta.rate)}/MIN
                </button>
              )}
              {c.cta?.kind === "trace" && (
                <button className="chip warn" onClick={() => act(c)} data-testid="card-cta">
                  RESOLVE…
                </button>
              )}
              {c.cta?.kind === "review" && (
                <button className="chip warn" onClick={() => act(c)} data-testid="card-cta">
                  REVIEW DRIFT
                </button>
              )}
              <button
                className="chip"
                onClick={() => void backend.advisorDismiss(c.id).then(setAdvisor)}
                data-testid="card-dismiss"
                title="Dismiss mutes this rule — it stops telling you about this"
              >
                DISMISS
              </button>
            </div>
            <footer className="advisor-provenance mono">
              SAW: {c.saw} @{c.at.slice(11, 16)} · RULE: {c.rule}
            </footer>
          </article>
        );
      })}
      {muted.length > 0 && (
        <div className="advisor-muted">
          <span className="t-label" style={{ color: "var(--ink-faint)" }}>
            MUTED RULES
          </span>
          {muted.map((rule) => (
            <button
              key={rule}
              className="chip"
              onClick={() => void backend.advisorUnmute(rule).then(setAdvisor)}
              data-testid={`unmute-${rule}`}
            >
              {rule} ×
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

interface ChatMsg {
  from: "user" | "engine";
  text: string;
  reply?: ChatReply;
}

function Chat() {
  const view = useStore((s) => s.view);
  const selection = useStore((s) => s.selection);
  const setSelection = useStore((s) => s.setSelection);
  const setView = useStore((s) => s.setView);
  const setReviewing = useStore((s) => s.setReviewing);
  const setAdvisorOpen = useStore((s) => s.setAdvisorOpen);
  const hydrate = useStore((s) => s.hydrate);
  const [scopeKind, setScopeKind] = useState<"empire" | "factory" | "selection">("empire");
  const [ctx, setCtx] = useState<ContextSnapshot | null>(null);
  const [showCtx, setShowCtx] = useState(false);
  const [msgs, setMsgs] = useState<ChatMsg[]>([]);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const logRef = useRef<HTMLDivElement>(null);

  const scope: ChatScope =
    scopeKind === "factory" && view.mode === "factory"
      ? { scope: "factory", id: view.factoryId }
      : scopeKind === "selection" && selection
        ? { scope: "selection", id: selection.id }
        : { scope: "empire" };

  const refreshCtx = useCallback(() => {
    backend
      .chatContext(scope)
      .then(setCtx)
      .catch(() => setCtx(null));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scopeKind, view, selection]);

  useEffect(() => {
    refreshCtx();
  }, [refreshCtx]);

  const send = async () => {
    const text = draft.trim();
    if (!text || busy) return;
    setDraft("");
    setBusy(true);
    setMsgs((m) => [...m, { from: "user", text }]);
    try {
      const reply = await backend.chatSend(scope, text);
      setMsgs((m) => [...m, { from: "engine", text: reply.reply, reply }]);
      if (reply.proposal) await hydrate(); // the drafted proposal is plan state
      requestAnimationFrame(() => logRef.current?.scrollTo(0, 1e6));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="advisor-body advisor-chat">
      <div className="chat-context mono">
        <select value={scopeKind} onChange={(e) => setScopeKind(e.target.value as typeof scopeKind)} className="mono">
          <option value="empire">EMPIRE</option>
          <option value="factory" disabled={view.mode !== "factory"}>
            FACTORY
          </option>
          <option value="selection" disabled={!selection}>
            SELECTION
          </option>
        </select>
        <span>{ctx ? `${(ctx.bytes / 1024).toFixed(1)} KB` : "—"}</span>
        <button className="chip" onClick={() => setShowCtx(!showCtx)} data-testid="btn-view-context">
          ▸ VIEW
        </button>
      </div>
      {showCtx && ctx && (
        <pre className="chat-ctx-json mono" data-testid="context-json">
          {JSON.stringify(ctx.payload, null, 1).slice(0, 4000)}
        </pre>
      )}
      <div className="chat-log" ref={logRef} data-testid="chat-log">
        {msgs.length === 0 && (
          <div className="drawer-empty" style={{ padding: 8 }}>
            Ask about "power" or "deficits", or say "produce Iron Rod at 30/min" — answers can propose
            changes; they arrive as proposals, never as edits.
          </div>
        )}
        {msgs.map((m, i) =>
          m.from === "user" ? (
            <div className="chat-user" key={i}>
              {m.text}
            </div>
          ) : (
            <div className="chat-answer" key={i} data-testid="chat-answer">
              <p>{m.text}</p>
              {m.reply && m.reply.causal.length > 0 && (
                <div className="chat-causal mono">
                  {m.reply.causal.map(([sev, line], j) => (
                    <div key={j} className={sev}>
                      {j + 1}. {line}
                    </div>
                  ))}
                </div>
              )}
              {m.reply && m.reply.entities.length > 0 && (
                <div className="chat-entities">
                  {m.reply.entities.map(([name, kind, id], j) => (
                    <button
                      key={j}
                      className="chip"
                      onClick={() => {
                        setView({ mode: "map" });
                        setSelection({ kind: kind as "factory", id });
                      }}
                    >
                      ◆ {name.toUpperCase()}
                    </button>
                  ))}
                </div>
              )}
              {m.reply?.proposal && (
                <button
                  className="chip warn"
                  onClick={() => {
                    setAdvisorOpen(false);
                    setReviewing(m.reply!.proposal!);
                  }}
                  data-testid="chat-review-proposal"
                >
                  REVIEW THE PROPOSAL
                </button>
              )}
              {m.reply && (
                <footer className="advisor-provenance mono">
                  SAW: {m.reply.saw} · ENGINE: {m.reply.engine.toUpperCase()}
                </footer>
              )}
            </div>
          ),
        )}
        {busy && <div className="mono chat-streaming">▉ WORKING…</div>}
      </div>
      <div className="chat-composer">
        <input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void send();
            e.stopPropagation();
          }}
          placeholder='e.g. "produce Iron Rod at 30/min"'
          data-testid="chat-input"
        />
        <button className="btn btn-primary" onClick={() => void send()} disabled={busy} data-testid="chat-send">
          SEND
        </button>
      </div>
    </div>
  );
}
