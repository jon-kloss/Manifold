import { describe, it, expect } from "vitest";
import { isPlainNode, isRenderableNode } from "./types";

// The two node gates must stay distinct: a fracking satellite RENDERS (and opens
// the well drawer) but is NOT plain (so the miner-claim path never touches it),
// and a geyser is neither until geothermal placement lands.
describe("node gates", () => {
  it("isPlainNode is true only for plain nodes", () => {
    expect(isPlainNode({ nodeType: "node" })).toBe(true);
    expect(isPlainNode({ nodeType: "fracking-satellite" })).toBe(false);
    expect(isPlainNode({ nodeType: "geyser" })).toBe(false);
  });

  it("isRenderableNode adds fracking satellites but not geysers", () => {
    expect(isRenderableNode({ nodeType: "node" })).toBe(true);
    expect(isRenderableNode({ nodeType: "fracking-satellite" })).toBe(true);
    expect(isRenderableNode({ nodeType: "geyser" })).toBe(false);
  });
});
