import { describe, it, expect } from "vitest";
import { isPlainNode, isRenderableNode } from "./types";

// The two node gates stay distinct: satellites and geysers RENDER (opening the
// well / geothermal drawer) but are NOT plain, so the miner-claim path only ever
// touches plain nodes.
describe("node gates", () => {
  it("isPlainNode is true only for plain nodes", () => {
    expect(isPlainNode({ nodeType: "node" })).toBe(true);
    expect(isPlainNode({ nodeType: "fracking-satellite" })).toBe(false);
    expect(isPlainNode({ nodeType: "geyser" })).toBe(false);
  });

  it("isRenderableNode covers plain nodes, fracking satellites, and geysers", () => {
    expect(isRenderableNode({ nodeType: "node" })).toBe(true);
    expect(isRenderableNode({ nodeType: "fracking-satellite" })).toBe(true);
    expect(isRenderableNode({ nodeType: "geyser" })).toBe(true);
  });
});
