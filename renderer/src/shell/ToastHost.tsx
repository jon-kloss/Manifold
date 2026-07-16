// Transient action-feedback toasts. Stacked bottom-center over the map; each
// auto-dismisses (timer armed in the store) and can be dismissed early by click.
// Feedback for actions the UI otherwise swallows — uploads, clipboard, etc.

import { useStore } from "../state/store";

const GLYPH = { success: "✓", error: "⚠", info: "›" } as const;

export default function ToastHost() {
  const toasts = useStore((s) => s.toasts);
  const dismiss = useStore((s) => s.dismissToast);
  if (toasts.length === 0) return null;
  return (
    <div className="toast-host" data-testid="toast-host" role="status" aria-live="polite">
      {toasts.map((t) => (
        <button
          key={t.id}
          className={`toast toast-${t.kind}`}
          data-testid="toast"
          onClick={() => dismiss(t.id)}
          title="Dismiss"
        >
          <span className="toast-glyph mono">{GLYPH[t.kind]}</span>
          <span className="toast-msg">{t.message}</span>
        </button>
      ))}
    </div>
  );
}
