import { describe, expect, it } from "vitest";
import { itemLabel, prettyClass, flowBand } from "./format";

describe("flowBand", () => {
  it("classifies a zero-flow belt as idle (not healthy green)", () => {
    expect(flowBand(0, 0)).toBe("idle");
    expect(flowBand(0.9, 0)).toBe("idle"); // saturation is moot when nothing flows
  });
  it("keeps the flowing bands", () => {
    expect(flowBand(0.3, 30)).toBe("under"); // ≤50% and flowing
    expect(flowBand(0.8, 48)).toBe("good"); // >50% and flowing
    expect(flowBand(1, 0, true)).toBe("bottleneck"); // solver evidence wins over idle
  });
});

describe("itemLabel", () => {
  it("prefers the catalog display name when present", () => {
    expect(itemLabel({ Desc_OreBauxite_C: { displayName: "Bauxite" } }, "Desc_OreBauxite_C")).toBe("Bauxite");
  });

  it("falls back to a known resource name when the catalog lacks the item", () => {
    // The bundled fixture doesn't carry every raw resource — a node must still
    // read "Bauxite", never the raw DESC_OREBAUXITE_C class.
    expect(itemLabel({}, "Desc_OreBauxite_C")).toBe("Bauxite");
    expect(itemLabel({}, "Desc_OreGold_C")).toBe("Caterium Ore");
    expect(itemLabel({}, "Desc_LiquidOil_C")).toBe("Crude Oil");
  });

  it("humanises an unknown class rather than showing it raw", () => {
    expect(itemLabel({}, "Desc_SomeModItem_C")).toBe("Some Mod Item");
    expect(itemLabel({}, "Desc_SomeModItem_C")).toBe(prettyClass("Desc_SomeModItem_C"));
  });

  it("returns empty string for an empty class (save-only nodes)", () => {
    expect(itemLabel({}, "")).toBe("");
  });
});
