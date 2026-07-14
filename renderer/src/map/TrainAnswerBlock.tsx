// Task #49 train answer-sheet block (read-only, presentational). The headline
// is TRAINS NEEDED = ceil(demand ÷ per-train throughput); below it the per-train
// figure, the demand (editable target when unknown), an honest SHORT/surplus
// verdict in flow tokens, and RTT/headway from the math. A COPY button writes
// the plain-text payload to the clipboard. Never mutates the plan.

import { useState } from "react";
import { fmtClockS, fmtKm, fmtRate } from "../lib/format";
import type { TrainAnswer } from "../state/types";
import { trainAnswerToText, UNIT, type TrainAnswerContext } from "./trainAnswer";

export default function TrainAnswerBlock({
  answer,
  ctx,
  onDemandChange,
}: {
  answer: TrainAnswer;
  ctx: TrainAnswerContext;
  /** When set, the DEMAND row becomes an editable target (prospective route). */
  onDemandChange?: (rate: number) => void;
}) {
  const [copied, setCopied] = useState(false);
  const unit = UNIT[ctx.kind];
  const hasDemand = answer.demandPerMin > 0;
  const level = !hasDemand ? "" : answer.short ? "warn" : "ok";
  const total = answer.perTrainPerMin * answer.trainsNeeded;

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(trainAnswerToText(answer, ctx));
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    } catch {
      setCopied(false);
    }
  };

  return (
    <div className="train-answer" data-testid="train-answer">
      <div className="ta-headline">
        <span className="t-label">TRAINS NEEDED</span>
        <span className={`ta-count mono ${level}`} data-testid="train-answer-count">
          {hasDemand ? `${answer.trainsNeeded}×` : "—"}
        </span>
        <span className="ta-unit mono">
          {unit}
          {answer.trainsNeeded === 1 ? "" : "s"}
        </span>
      </div>

      <div className="math-block mono ta-math">
        <div className="math-row">
          <span>PER {unit.toUpperCase()}</span>
          <span className="math-note">{fmtKm(answer.math.effectiveLengthM)}</span>
          <span className="projected">{fmtRate(answer.perTrainPerMin)}/min</span>
        </div>
        <div className="math-row">
          <span>DEMAND</span>
          <span className="math-note" />
          {onDemandChange ? (
            <span>
              <input
                type="number"
                className="mono math-edit ta-demand"
                min={0}
                step={10}
                value={Math.round(answer.demandPerMin)}
                onChange={(e) => onDemandChange(Math.max(0, Number(e.target.value)))}
                data-testid="train-answer-demand"
              />
              <span className="unit">/min</span>
            </span>
          ) : (
            <span className="projected">{hasDemand ? `${fmtRate(answer.demandPerMin)}/min` : "—"}</span>
          )}
        </div>
        {hasDemand && (
          <div className={`math-row math-total ${level}`} data-testid="train-answer-verdict">
            <span>RESULT</span>
            <span className="math-note" />
            <span className="projected">
              {answer.short
                ? `SHORT by ${fmtRate(answer.demandPerMin - answer.math.throughputPerMin)}/min ⚠`
                : `${fmtRate(total)}/min · +${fmtRate(total - answer.demandPerMin)} ✓`}
            </span>
          </div>
        )}
        <div className="math-row">
          <span>RTT</span>
          <span className="math-note">round trip {fmtClockS(answer.math.roundTripS)}</span>
          <span>{fmtClockS(answer.math.rttS)}</span>
        </div>
        {answer.math.headwayS != null && (
          <div className="math-row">
            <span>HEADWAY</span>
            <span className="math-note" />
            <span>{fmtClockS(answer.math.headwayS)}</span>
          </div>
        )}
      </div>

      <button className="btn btn-ghost ta-copy" onClick={() => void copy()} data-testid="btn-train-answer-copy">
        {copied ? "COPIED ✓" : "COPY ANSWER"}
      </button>
    </div>
  );
}
