// Dismiss a popup when the user clicks OUTSIDE it or presses Escape. The
// graph-toolbar dropdowns (+ LOGISTIC, + MACHINE, + IN/OUT PORT) previously
// only closed on an explicit pick or re-toggle, so an abandoned menu stuck
// around over the canvas. mousedown (not click) so the popup is gone before
// the outside interaction's own click lands; capture phase so a stopPropagation
// in the underlying surface can't keep the menu alive.

import { useEffect, type RefObject } from "react";

export function useDismiss(
  ref: RefObject<HTMLElement | null>,
  onClose: () => void,
  active = true,
): void {
  useEffect(() => {
    if (!active) return;
    const onDown = (e: MouseEvent) => {
      const el = ref.current;
      if (el && e.target instanceof Node && !el.contains(e.target)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        // consumed: closing the popup must not also clear the selection /
        // eject to the map (the views' own Escape handlers run on bubble)
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("mousedown", onDown, true);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("mousedown", onDown, true);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [ref, onClose, active]);
}
