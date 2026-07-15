// Shared e2e helpers. The suite runs serially against ONE dev-bridge and ONE
// plan file, so a spec that dies mid-run (e.g. a dropped drag gesture) leaves
// its persisted viewState behind for every spec that follows — the classic
// cascade: exit-criterion dies in graph view → the next spec boots into
// `mode: "factory"` and times out waiting for `map-root`, or the resume
// dashboard auto-presents and its scrim swallows the first right-drag.

import type { APIRequestContext } from "@playwright/test";

const API = "http://localhost:8791/api";

/**
 * Deterministic map boot for API-seeding specs — call BEFORE the first
 * page.goto. POST /api/view REPLACES the stored viewState blob wholesale
 * (plan_file set_meta), and the replace is the point:
 *
 *  - `openFactory`/`mode` are cleared → the app boots to the map, never into
 *    a predecessor's abandoned graph view;
 *  - `resumeSeen` lands spent (true) → the resume dashboard cannot ambush the
 *    spec with its `.dash-scrim` over the map (it swallows right-drag
 *    mousedown);
 *  - `onboarded` is wiped, which is inert here: Onboarding gates on an EMPTY
 *    plan (Onboarding.tsx), and every spec using this helper runs against a
 *    plan that earlier serial specs (or its own API seeding) populated.
 *
 * This decouples each spec from its serial predecessor exiting cleanly —
 * standalone and subset runs become valid. exit-criterion is the deliberate
 * exception (it asserts the resume-dashboard lifecycle, so it needs the
 * pristine unspent flag and carries its own defensive prologue instead).
 */
export async function resetView(request: APIRequestContext): Promise<void> {
  const res = await request.post(`${API}/view`, { data: JSON.stringify({ resumeSeen: true }) });
  if (!res.ok()) throw new Error(`resetView ${res.status()}: ${await res.text()}`);
}
