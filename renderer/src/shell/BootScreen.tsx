// MANIFOLD boot screen (brand handoff §4a/§6/§7): the expanding manifold IS
// the progress bar. Full-screen on the map's base canvas color — the splash
// → map transition (motion 7j) is a single dissolve on that shared backdrop
// (see boot.css for why there is deliberately no grid here). Choreography:
//   1. Survey intro (~1.1s, wall time — no data needed yet): crosshair sweep,
//      the source diamond drops in with a survey ring.
//   2. Expansion, progress-driven: the bus extends symmetrically with the
//      REAL load fraction (store.boot, smoothed + strictly monotonic in
//      bootSim); sinks land at their bus offsets; item dots ride only
//      completed legs ON WALL TIME so stalls breathe instead of freezing.
//   3. Done-beat (~1.7s): MANIFOLD wordmark rises letter-by-letter, ticker
//      reads the final stage — then a 0.5s crossfade reveals the live map
//      (the app frame plays .boot-reveal's 1.05 → 1 scale-in beneath).
// FAST-BOOT: §7 pins the survey intro to wall time ("no data needed yet"),
// so it always plays in full even when a warm local plan hydrates in tens
// of ms — the expansion then closes fast from the already-complete fraction.
// The animated splash then holds for a MINIMUM wall time (MIN_SPLASH_S) before
// revealing, so the choreography — expansion, wordmark rising letter-by-letter,
// a short done-beat — always plays in full instead of a warm boot flashing past.
// That floor SCALES UP with empire size (bigger plan → more to take in →
// PER_FACTORY_SPLASH_S per factory, capped at MAX_SPLASH_S). The full DONE_BEAT
// still plays whenever loading genuinely outlives the intro (web boot, big saves).
// prefers-reduced-motion: a static mark + live ticker, dismissed the moment
// the app is ready — same contract the map animation loop honors; no floor.
// Glow is 3 layered strokes + a radial-gradient bloom — no CSS filters.

import { useEffect, useRef, useState } from "react";
import { useStore } from "../state/store";
import {
  BUS_Y,
  CX,
  HALF,
  LEGS,
  OFFS,
  SINK_Y,
  SRC_Y,
  TAP_Y,
  back,
  c01,
  eoc,
  legLive,
  legPoint,
  minSplashSeconds,
  newBootSim,
  seg,
  stepBootSim,
} from "./bootSim";
import "./boot.css";

const SURVEY_S = 1.1;
const DONE_BEAT_S = 1.7;
// The warm-boot done-beat: long enough for the wordmark to rise letter-by-letter
// (finishes at done ≈ 0.58) and hold a beat, so the animation is seen — not the
// old 0.25s pop that revealed almost immediately.
const FAST_BEAT_S = 0.8;
const REVEAL_S = 0.5;

export type BootPhase = "load" | "reveal" | "done";

function GlowLine({ d, op = 1 }: { d: string; op?: number }) {
  return (
    <g>
      <path d={d} fill="none" stroke="var(--signal-500)" strokeWidth="11" opacity={0.07 * op} />
      <path d={d} fill="none" stroke="var(--signal-500)" strokeWidth="5" opacity={0.2 * op} />
      <path d={d} fill="none" stroke="var(--signal-400)" strokeWidth="2.4" opacity={0.85 * op} />
    </g>
  );
}

const dia = (x: number, y: number, r: number) => `M${x} ${y - r} L${x + r} ${y} L${x} ${y + r} L${x - r} ${y} Z`;

function Diamond({
  x,
  y,
  r,
  op = 1,
  scale = 1,
  glow = 0,
}: {
  x: number;
  y: number;
  r: number;
  op?: number;
  scale?: number;
  glow?: number;
}) {
  if (op <= 0 || scale <= 0) return null;
  return (
    <g opacity={op}>
      {glow > 0.01 && <circle cx={x} cy={y} r={r * 3.4} fill="url(#mfBloom)" opacity={glow} />}
      <path d={dia(x, y, Math.max(0.01, r * scale))} fill="var(--signal-500)" />
      <path d={dia(x, y, Math.max(0.01, r * scale))} fill="none" stroke="var(--signal-400)" strokeWidth="1.5" opacity="0.6" />
    </g>
  );
}

export default function BootScreen({
  phase,
  setPhase,
}: {
  phase: BootPhase;
  setPhase: (p: BootPhase) => void;
}) {
  const boot = useStore((s) => s.boot);
  const ready = useStore((s) => s.ready);
  const error = useStore((s) => s.error);
  const reduced =
    typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  // The sim lives in a ref; a ~30fps state mirror drives React. Boot target
  // rides a ref too so the rAF loop always sees the latest loader fraction.
  const simRef = useRef(newBootSim());
  const targetRef = useRef(0);
  targetRef.current = boot.fraction;
  const [, force] = useState(0);
  const phaseRef = useRef(phase);
  phaseRef.current = phase;
  const revealAtRef = useRef(-1);
  const fastRef = useRef(false);

  // Backend failure: the refusal card owns the screen — no beats, no fade.
  useEffect(() => {
    if (error) setPhase("done");
  }, [error, setPhase]);

  // Reduced motion: dismiss the moment the app is ready (no forced beats).
  useEffect(() => {
    if (reduced && ready) setPhase("done");
  }, [reduced, ready, setPhase]);

  useEffect(() => {
    if (reduced) return;
    let raf = 0;
    let last = performance.now();
    const setPhaseSafe = (p: BootPhase) => {
      if (phaseRef.current !== p) setPhase(p);
    };
    const loop = (now: number) => {
      const dt = Math.min(0.05, (now - last) / 1000);
      last = now;
      const sim = simRef.current;
      // Survey runs on wall time (§7) and always plays in full; expansion
      // consumes the loader fraction. A loader that finishes inside the
      // intro window only marks the boot fast (short done-beat) — it does
      // not cut the sweep/lock short.
      if (sim.t < SURVEY_S && targetRef.current >= 1) fastRef.current = true;
      stepBootSim(sim, dt, sim.t < SURVEY_S ? 0 : targetRef.current);
      const beat = fastRef.current ? FAST_BEAT_S : DONE_BEAT_S;
      const st = useStore.getState();
      // Hold the splash for at least a floor that grows with empire size, so the
      // animation plays in full and a warm boot no longer flashes past. A
      // genuinely slow load already outlives this floor, so it's a no-op there.
      const minSplash = minSplashSeconds(Object.keys(st.plan.factories).length);
      if (phaseRef.current === "load" && sim.done >= beat && st.ready && sim.t >= minSplash) {
        revealAtRef.current = sim.t;
        setPhaseSafe("reveal");
      }
      if (phaseRef.current === "reveal" && sim.t - revealAtRef.current >= REVEAL_S) {
        setPhaseSafe("done");
        return; // unmounting — stop the loop
      }
      force((n) => n + 1);
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(raf);
  }, [reduced, setPhase]);

  if (reduced) {
    // Static frame: mark + live ticker, no choreography.
    return (
      <div className="boot-screen" data-testid="boot-screen">
        <div className="boot-center">
          <svg viewBox="0 0 64 64" width="72" height="72" aria-hidden>
            <path
              d="M32 14 V26 M14 26 H50 M14 26 V40 M32 26 V40 M50 26 V40"
              fill="none"
              stroke="var(--signal-500)"
              strokeWidth="6"
              opacity="0.6"
            />
            <path d="M32 0 L42 10 L32 20 L22 10 Z" fill="var(--signal-500)" />
            <path d="M14 37 L22 45 L14 53 L6 45 Z" fill="var(--signal-500)" />
            <path d="M32 37 L40 45 L32 53 L24 45 Z" fill="var(--signal-500)" />
            <path d="M50 37 L58 45 L50 53 L42 45 Z" fill="var(--signal-500)" />
          </svg>
          <div className="boot-wordmark">MANIFOLD</div>
          <div className="boot-ticker mono" data-testid="boot-ticker">
            {boot.stage}
          </div>
        </div>
      </div>
    );
  }

  const sim = simRef.current;
  const { t, prog, done, land } = sim;
  const surveyP = c01(t / SURVEY_S);
  const inSurvey = t < SURVEY_S;
  const revealP = phase === "reveal" ? c01((t - revealAtRef.current) / REVEAL_S) : 0;
  const splashOp = 1 - revealP;

  // Survey geometry (crosshair sweep locking the site).
  const chOp = 0.35 * seg(surveyP, 0.02, 0.14) * (1 - seg(surveyP, 0.44, 0.58));
  const sweep = eoc(seg(surveyP, 0.02, 0.4));
  const lx = 320 + (CX - 320) * sweep;
  const ly = 560 + (SRC_Y - 560) * sweep;
  const srcOp = inSurvey ? seg(surveyP, 0.3, 0.42) : 1;
  const srcY = inSurvey ? SRC_Y - 110 * (1 - eoc(seg(surveyP, 0.3, 0.6))) : SRC_Y;
  const ringP = seg(surveyP, 0.6, 0.95);

  // Expansion geometry.
  const feedP = inSurvey ? 0 : eoc(c01((t - SURVEY_S) / 0.3));
  const tip = HALF * prog;
  const breath = 0.35 + 0.09 * Math.sin(t * 2.4);
  // Always rise letter-by-letter (driven by `done`) — the warm-boot floor gives
  // the beat room to play, so there's no need to pop the wordmark instantly.
  const wordLetter = (i: number) => seg(done, 0.05 + i * 0.04, 0.3 + i * 0.04);

  return (
    <div className="boot-screen" data-testid="boot-screen" style={{ opacity: splashOp }}>
      <svg className="boot-stage" viewBox="0 0 1280 720" preserveAspectRatio="xMidYMid meet" aria-hidden>
        <defs>
          <radialGradient id="mfBloom">
            <stop offset="0%" stopColor="var(--signal-400)" stopOpacity="0.5" />
            <stop offset="40%" stopColor="var(--signal-500)" stopOpacity="0.16" />
            <stop offset="100%" stopColor="var(--signal-500)" stopOpacity="0" />
          </radialGradient>
        </defs>
        {inSurvey && (
          <g opacity={chOp}>
            <line x1="0" y1={ly} x2="1280" y2={ly} stroke="var(--ink-faint)" strokeWidth="1" />
            <line x1={lx} y1="0" x2={lx} y2="720" stroke="var(--ink-faint)" strokeWidth="1" />
            <rect x={lx - 7} y={ly - 7} width="14" height="14" fill="none" stroke="var(--ink-faint)" strokeWidth="1" />
          </g>
        )}
        {inSurvey && ringP > 0 && ringP < 1 && (
          <circle
            cx={CX}
            cy={SRC_Y}
            r={90 * eoc(ringP)}
            fill="none"
            stroke="var(--signal-500)"
            strokeWidth="2"
            opacity={0.5 * (1 - ringP)}
          />
        )}
        {!inSurvey && (
          <>
            {feedP > 0 && <GlowLine d={`M${CX} ${SRC_Y + 20} V${SRC_Y + 20 + (BUS_Y - SRC_Y - 20) * feedP}`} />}
            {prog > 0.004 && <GlowLine d={`M${CX} ${BUS_Y} H${CX - tip}`} />}
            {prog > 0.004 && <GlowLine d={`M${CX} ${BUS_Y} H${CX + tip}`} />}
            {prog > 0.004 &&
              prog < 0.999 &&
              [-1, 1].map((s) => (
                <g key={s}>
                  <line
                    x1={CX + s * Math.max(0, tip - 36)}
                    y1={BUS_Y}
                    x2={CX + s * tip}
                    y2={BUS_Y}
                    stroke="var(--signal-400)"
                    strokeWidth="3.5"
                    opacity="0.4"
                  />
                  <circle cx={CX + s * tip} cy={BUS_Y} r="4.5" fill="var(--signal-comet)" />
                </g>
              ))}
            {OFFS.map((off, i) => {
              if (land[i] < 0) return null;
              const a = t - land[i];
              const pop = c01(a / 0.35);
              const tapP = eoc(c01(a / 0.3));
              const flash = c01(1 - a / 0.9);
              const ring = c01(a / 0.7);
              const xs = off === 0 ? [CX] : [CX - off, CX + off];
              return xs.map((x) => (
                <g key={x}>
                  <GlowLine d={`M${x} ${BUS_Y} V${BUS_Y + (TAP_Y - BUS_Y) * tapP}`} />
                  {ring > 0 && ring < 1 && (
                    <circle
                      cx={x}
                      cy={SINK_Y}
                      r={36 * eoc(ring)}
                      fill="none"
                      stroke="var(--signal-500)"
                      strokeWidth="1.5"
                      opacity={0.45 * (1 - ring)}
                    />
                  )}
                  <Diamond x={x} y={SINK_Y} r={16} scale={Math.min(1.25, back(pop))} glow={0.5 * flash} />
                </g>
              ));
            })}
            {LEGS.map((off, i) => {
              if (!legLive(sim, off)) return null;
              const u = (t / 1.5 + i * 0.137) % 1;
              return (
                <g key={off}>
                  {[0, 0.035, 0.07].map((lag, k) => {
                    const uu = u - lag;
                    if (uu <= 0 || uu >= 1) return null;
                    const [x, y] = legPoint(off, uu);
                    return (
                      <circle
                        key={k}
                        cx={x}
                        cy={y}
                        r={[3.4, 2.3, 1.4][k]}
                        fill={k === 0 ? "var(--signal-comet)" : "var(--signal-400)"}
                        opacity={[1, 0.5, 0.25][k]}
                      />
                    );
                  })}
                </g>
              );
            })}
          </>
        )}
        <Diamond x={CX} y={srcY} r={20} op={srcOp} glow={inSurvey ? 0.35 * seg(surveyP, 0.55, 0.8) : breath} />
      </svg>
      {inSurvey && (
        <div className="boot-survey-chip mono" style={{ opacity: seg(surveyP, 0.72, 0.8) * (1 - seg(surveyP, 0.9, 0.98)) }}>
          SURVEY LOCK
        </div>
      )}
      {!inSurvey && (
        <div className="boot-center boot-under">
          <div className="boot-wordmark" aria-hidden={done <= 0}>
            {"MANIFOLD".split("").map((ch, i) => {
              const e = eoc(wordLetter(i));
              return (
                <span key={i} style={{ opacity: e, transform: `translateY(${(1 - e) * 16}px)`, display: "inline-block" }}>
                  {ch}
                </span>
              );
            })}
          </div>
          <div className="boot-ticker mono" data-testid="boot-ticker">
            {boot.stage}
            <span className="boot-cursor" style={{ opacity: t % 1 < 0.5 ? 1 : 0 }} />
          </div>
        </div>
      )}
      <div className="boot-vignette" />
    </div>
  );
}
