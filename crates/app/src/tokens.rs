//! Design tokens — the single source (UI spec §Design Tokens, verbatim).
//! `gen-tokens` renders these to renderer/src/tokens/tokens.css + tokens.ts;
//! CI fails if the generated files drift. No hex value ships outside this file.

pub struct Token {
    pub name: &'static str,
    pub value: &'static str,
}

macro_rules! tokens {
    ($( $name:literal => $value:literal ),* $(,)?) => {
        &[ $( Token { name: $name, value: $value } ),* ]
    };
}

/// Colors. Rules that are law: "orange is a verb" (Signal = interactive or
/// attention only), Blueprint blue = planned only, Flow colors = load/status only.
pub const COLORS: &[Token] = tokens![
    // Steel — the only surfaces, no gradients on chrome
    "steel-950" => "#0E1012",
    "steel-900" => "#15181B",
    "steel-800" => "#1C2024",
    "steel-700" => "#262B30",
    "steel-600" => "#333A41",
    "steel-500" => "#49525B",
    "map-canvas" => "#0D1013",
    "map-grid" => "#14181C",
    "graph-dot" => "#1D2126",
    // Ink
    "ink-100" => "#ECEEF0",
    "ink-300" => "#AAB3BB",
    "ink-500" => "#6C7680",
    "ink-faint" => "#49525B",
    "ink-ghost" => "#3B434B",
    // Signal — orange is a verb
    "signal-500" => "#F78B23",
    "signal-400" => "#FFA347",
    "signal-pressed" => "#D97614",
    "signal-selected-bg" => "rgba(247,139,35,.14)",
    "signal-selected-border" => "rgba(247,139,35,.35)",
    "on-signal" => "#141414",
    // Blueprint — exclusively "planned"; nothing else may be blue
    "bp-400" => "#56A8FF",
    "bp-600" => "#2A5F96",
    "bp-ghost" => "rgba(86,168,255,.08)",
    "bp-dim-text" => "#5A86B3",
    // Flow — saturation/status, always paired with a redundant channel
    "flow-ok" => "#46C07A",
    "flow-ok-dark" => "#1E4531",
    "flow-warn" => "#E5A83B",
    "flow-warn-dark" => "#8A6423",
    "flow-crit" => "#FF5D55",
    "flow-crit-dark" => "#7A2622",
    "underclock" => "#5BC8C0",
    "underclock-dark" => "#2A5F57",
];

pub const TYPOGRAPHY: &[Token] = tokens![
    "font-display" => "'Rajdhani', 'Arial Narrow', sans-serif",
    "font-body" => "'Barlow', 'Helvetica Neue', sans-serif",
    "font-mono" => "'JetBrains Mono', 'SFMono-Regular', Consolas, monospace",
];

pub const CHROME: &[Token] = tokens![
    "titlebar-h" => "36px",
    "statusbar-h" => "24px",
    "contextbar-h" => "36px",
    "radius" => "3px",
    "row-dense" => "24px",
    "row-default" => "28px",
    // Blueprint hatch (planned fill) — UI spec exact recipe
    "bp-hatch" => "repeating-linear-gradient(45deg, rgba(86,168,255,.18) 0 2px, transparent 2px 5px)",
    // Signature corner cut (proposal cards, modals, primary CTAs only)
    "corner-cut" => "polygon(0 0, calc(100% - 8px) 0, 100% 8px, 100% 100%, 0 100%)",
];

pub const MOTION: &[Token] = tokens![
    "ease" => "cubic-bezier(.2,0,0,1)",
    "dur-hover" => "120ms",
    "dur-drawer" => "200ms",
    "dur-settle" => "300ms",
    "dur-morph" => "400ms",
];

/// A1 responsive breakpoints + docked panel widths (full/compact).
pub const LAYOUT: &[Token] = tokens![
    "bp-floor" => "1366px",
    "bp-compact" => "1600px",
    "bp-reference" => "1920px",
    "w-inspector" => "360px",
    "w-inspector-compact" => "320px",
    "w-advisor" => "340px",
    "w-advisor-compact" => "300px",
    "w-proposal" => "470px",
    "w-proposal-compact" => "420px",
    "w-summary" => "380px",
    "w-summary-compact" => "340px",
    "w-recipe-strip" => "640px",
    "w-recipe-strip-compact" => "560px",
    "w-recipe-strip-overlay" => "520px",
];

pub fn all() -> Vec<(&'static str, &'static Token)> {
    let mut out = Vec::new();
    for (group, list) in [
        ("color", COLORS),
        ("type", TYPOGRAPHY),
        ("chrome", CHROME),
        ("motion", MOTION),
        ("layout", LAYOUT),
    ] {
        for t in list.iter() {
            out.push((group, t));
        }
    }
    out
}
