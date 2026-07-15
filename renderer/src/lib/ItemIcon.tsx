// A recognizable item tile: the vendored community icon when we ship one
// (renderer/public/icons, see NOTICE.md), otherwise the deterministic
// colour + monogram chip. Keeps the s20/s28/s40 size vocabulary of the
// placeholder it supersedes. Degradation is honest: a class outside the
// manifest — or an <img> that fails to load — renders the monogram chip,
// never a broken image glyph.
import { useState, type CSSProperties } from "react";
import { itemAccent, itemMonogram } from "./itemChip";
import iconManifest from "./iconManifest.json";

/** The vendored-icon manifest as a set — the single "do we ship this icon?"
 *  authority. Exported so other icon consumers (e.g. the footprint strip's
 *  machine render) gate on it instead of firing guaranteed-404 requests. */
export const ICONS: ReadonlySet<string> = new Set<string>(iconManifest);

export default function ItemIcon({
  item,
  displayName,
  size = 20,
}: {
  item: string;
  displayName?: string;
  size?: 20 | 28 | 40;
}) {
  // Keyed by item class so a failed load only benches THAT icon — the same
  // component instance re-tries when a caller swaps the item prop.
  const [failedItem, setFailedItem] = useState<string | null>(null);
  // Save-only / unknown items carry item:"" — degrade to a neutral tile.
  if (!item) return <span className={`item-chip s${size}`} aria-hidden />;
  const hasIcon = ICONS.has(item) && failedItem !== item;
  return (
    <span
      className={`item-chip s${size}`}
      style={{ "--chip-accent": itemAccent(item) } as CSSProperties}
      title={displayName ?? item}
      aria-hidden
    >
      {hasIcon ? (
        <img
          src={`/icons/${item}.png`}
          width={size}
          height={size}
          alt=""
          draggable={false}
          onError={() => setFailedItem(item)}
        />
      ) : (
        itemMonogram(item, displayName)
      )}
    </span>
  );
}
