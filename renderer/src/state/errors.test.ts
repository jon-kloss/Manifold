import { describe, it, expect } from "vitest";
import { friendlyError } from "./errors";

describe("friendlyError", () => {
  it("explains built-immutable without leaking the id, and names the action", () => {
    const out = friendlyError("built entities are immutable: 01KXV24XXCH4S3SRQEJBEHWSHV (set tier)");
    expect(out).toMatch(/can't set tier/i);
    expect(out).toMatch(/imported as built/i);
    expect(out).toMatch(/rebuild it at the new value/i); // value edit → rebuild advice
    expect(out).not.toMatch(/01KXV24XXCH4S3SRQEJBEHWSHV/); // no raw ULID
  });

  it("uses neutral advice for structural built actions (not 'rebuild at the new value')", () => {
    const out = friendlyError("built entities are immutable: 01ABC (delete)");
    expect(out).toMatch(/can't delete/i);
    expect(out).not.toMatch(/at the new value/i);
    expect(out).toMatch(/in-game.*re-import/i);
  });

  it("softens entity-not-found", () => {
    expect(friendlyError("entity not found: 01ABC")).toMatch(/no longer exists/i);
  });

  it("maps a known invalid rule to actionable copy", () => {
    expect(friendlyError("invalid value: these factories are already connected")).toMatch(
      /already share a power line/i,
    );
    expect(friendlyError("invalid value: a port is already bound to a route")).toMatch(/release the existing route/i);
  });

  it("drops the 'invalid value:' prefix and sentence-cases unmapped rules", () => {
    expect(friendlyError("invalid value: belt tier 7 outside Mk.1–Mk.6")).toBe("Belt tier 7 outside Mk.1–Mk.6.");
  });

  it("passes through anything it doesn't recognize", () => {
    expect(friendlyError("some other backend failure")).toBe("some other backend failure");
  });
});
