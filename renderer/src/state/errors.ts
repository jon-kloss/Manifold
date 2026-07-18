// Turn a raw DomainError string from the core into user-facing copy — say what
// is wrong and, where we can, how to fix it — instead of surfacing the core's
// terse prose (or a raw entity id). Pure; the store runs it before nameIds so
// any ids left in a passed-through message still resolve to display names.

const invalidHints: [RegExp, string][] = [
  [/already connected/i, "These factories already share a power line — only one is allowed."],
  [/already bound to a route/i, "That port already feeds a route. Release the existing route first, or pick a different port."],
  [/run from an OUT port to an IN port/i, "A belt route runs from an OUT port to an IN port — pick one of each."],
  [/ports carry different items/i, "Those ports carry different items — a route can only join matching items."],
  [/belongs to a different factory/i, "Both ends of a belt must live in the same factory."],
  [/needs two different endpoints|needs two different factories/i, "Pick two different endpoints — a link can't start and end in the same place."],
  [/all \d+ .* ports connected/i, "Every port on that machine is already wired — add a port or free one up first."],
  [/already carries a different item/i, "That port already carries a different item. Use a separate port for this one."],
];

export const friendlyError = (raw: string): string => {
  const built = raw.match(/^built entities are immutable: \S+ \((.+)\)$/);
  if (built) {
    const action = built[1];
    // Value edits (tier/clock/count/…) want "rebuild at the new value"; structural
    // actions (delete/move/expand) want a neutral "change it in-game" instead.
    const valueEdit = /tier|rate|clock|count|recipe|ceiling|floor|spec|priori/i.test(action);
    const tail = valueEdit
      ? "Rebuild it at the new value in-game, then re-import to apply the change."
      : "Make the change in-game, then re-import your save.";
    return `Can't ${action} — this is imported as built, so it's fixed to your save. ${tail}`;
  }
  if (/^entity not found:/.test(raw)) {
    return "That item no longer exists — it may have just been deleted or undone. Nothing was changed.";
  }
  const invalid = raw.match(/^invalid value: (.+)$/);
  if (invalid) {
    const inner = invalid[1];
    for (const [re, hint] of invalidHints) if (re.test(inner)) return hint;
    // Unmapped rules read fine on their own — just drop the "invalid value:"
    // jargon prefix and sentence-case them.
    return inner.charAt(0).toUpperCase() + inner.slice(1) + (/[.!?]$/.test(inner) ? "" : ".");
  }
  return raw;
};
