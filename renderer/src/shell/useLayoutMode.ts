// Addendum A1 — responsive degradation. The canvas is the only flex element;
// panels have exactly two docked widths plus an overlay conversion. All CSS px.

import { useEffect, useState } from "react";

export type LayoutMode = "reference" | "compact" | "overlay" | "refuse";

export function layoutModeFor(width: number, height: number): LayoutMode {
  if (width < 1366 || height < 768) return "refuse";
  if (width < 1600) return "overlay";
  if (width < 1920) return "compact";
  return "reference";
}

export function useLayoutMode(): { mode: LayoutMode; width: number; height: number } {
  const [size, setSize] = useState({ width: window.innerWidth, height: window.innerHeight });
  useEffect(() => {
    const onResize = () => setSize({ width: window.innerWidth, height: window.innerHeight });
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);
  return { mode: layoutModeFor(size.width, size.height), ...size };
}
