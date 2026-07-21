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
    // Flow — status only, always paired with a redundant channel. Efficiency
    // grammar (DECISIONS): flow-ok = GOOD (>50% utilized, including a FULL
    // belt meeting demand — optimal), flow-warn = UNDER-USED (flowing ≤50%:
    // over-built or starved upstream), flow-crit = BOTTLENECK (the link
    // provably caps demanded throughput — solver-named, never a bare % cut).
    // Hexes unchanged from the congestion era; only the meanings moved.
    "flow-ok" => "#46C07A",
    "flow-ok-dark" => "#1E4531",
    "flow-warn" => "#E5A83B",
    "flow-warn-dark" => "#8A6423",
    "flow-crit" => "#FF5D55",
    "flow-crit-dark" => "#7A2622",
    "underclock" => "#5BC8C0",
    "underclock-dark" => "#2A5F57",
    // MANIFOLD boot choreography (handoff §6): the comet head riding the
    // growing bus + item dots. Loader-only decorative color — "orange is a
    // verb" stays intact (the loader IS the sanctioned decorative surface);
    // the surveyor crosshair reuses ink-faint.
    "signal-comet" => "#FFD9AE",
    // Resource identity — MAP DATA ONLY, never a UI signal. Muted, low-chroma
    // terrain tints so a player can read node type at a glance (iron vs copper
    // vs oil) without competing with the reserved verbs (signal orange, plan
    // blue, flow status). Keyed by extracted resource; purity stays the ring.
    "resource-iron" => "#6E7D8C",
    "resource-copper" => "#B0663C",
    "resource-limestone" => "#B3A078",
    "resource-coal" => "#4A5057",
    "resource-caterium" => "#736026",
    "resource-quartz" => "#C07EA8",
    "resource-sulfur" => "#B9BE4A",
    "resource-oil" => "#7C67B0",
    "resource-bauxite" => "#B57C5E",
    "resource-uranium" => "#4F7A45",
    "resource-sam" => "#96587F",
    "resource-water" => "#2E6C9E",
    "resource-nitrogen" => "#2C737E",
    "resource-geyser" => "#B24A2E",
    "resource-generic" => "#5A6570",
    // MANIFOLD low-poly ore facets (brand handoff §3): -light/-dark shade the
    // faceted chunk, -hi is the specular highlight/sparkle/glow. Coal and
    // caterium icons facet from a brighter -mid base than the muted map tint
    // above (the disc tint stays the terrain color; the icon reads material).
    "resource-iron-light" => "#9AA8B6",
    "resource-iron-dark" => "#47525D",
    "resource-copper-light" => "#DB9058",
    "resource-copper-dark" => "#7A4224",
    "resource-copper-hi" => "#F0B584",
    "resource-limestone-light" => "#D6C49C",
    "resource-limestone-dark" => "#7E6E4E",
    "resource-coal-mid" => "#5C646C",
    "resource-coal-light" => "#99A3AD",
    "resource-coal-dark" => "#30353B",
    "resource-caterium-mid" => "#A98E39",
    "resource-caterium-light" => "#D9B94E",
    "resource-caterium-dark" => "#4E4119",
    "resource-caterium-hi" => "#F4E7AE",
    "resource-quartz-light" => "#E7B3D1",
    "resource-quartz-dark" => "#8A5578",
    "resource-quartz-hi" => "#F6DCEB",
    "resource-sulfur-light" => "#DCE070",
    "resource-sulfur-dark" => "#83872F",
    "resource-oil-dark" => "#55437E",
    "resource-oil-hi" => "#D3C8EC",
    "resource-bauxite-light" => "#D8A181",
    "resource-bauxite-dark" => "#7E5138",
    "resource-uranium-dark" => "#33502D",
    "resource-uranium-glow" => "#9FE58A",
    "resource-uranium-glow-dim" => "#6FBF5C",
    "resource-sam-light" => "#C084AB",
    "resource-sam-dark" => "#663A57",
    "resource-sam-glow" => "#ECC2DC",
    "resource-generic-light" => "#838F9B",
    "resource-generic-dark" => "#3B434C",
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

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(hex: &str, i: usize) -> f64 {
        u8::from_str_radix(&hex[1 + i * 2..3 + i * 2], 16).expect("token is #RRGGBB hex") as f64
            / 255.0
    }

    /// WCAG 2.x relative luminance of an sRGB #RRGGBB hex.
    fn relative_luminance(hex: &str) -> f64 {
        let lin = |c: f64| {
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(channel(hex, 0))
            + 0.7152 * lin(channel(hex, 1))
            + 0.0722 * lin(channel(hex, 2))
    }

    /// WCAG contrast ratio, (L_hi + 0.05) / (L_lo + 0.05).
    fn contrast(a: &str, b: &str) -> f64 {
        let (la, lb) = (relative_luminance(a), relative_luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    fn color(name: &str) -> &'static str {
        COLORS
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("missing color token {name}"))
            .value
    }

    /// Palette law: resource identity fills may never camouflage the signal
    /// marks drawn over them on the map (claim dot, conflict ring). Every
    /// resource-* fill must hold a ≥1.5:1 luminance contrast against each
    /// mark color. Marks exclude the -dark variants (flow-ok-dark etc.) —
    /// the dark variants are fills' darker edge shades, never adjacent to
    /// signal marks.
    #[test]
    fn resource_fills_keep_signal_marks_legible() {
        let marks = ["signal-500", "flow-ok", "flow-warn", "flow-crit"];
        // Legacy fills that predate the ≥1.5 law (worst: quartz vs flow-crit
        // ≈1.0). They rely on the canvas-bg keylines under map marks; this
        // list may only shrink — never add a new token to it.
        let legacy = [
            "resource-iron",
            "resource-copper",
            "resource-limestone",
            "resource-quartz",
            "resource-sulfur",
            "resource-bauxite",
        ];
        // MANIFOLD facet shades (-light/-mid/-hi/-glow…) are icon-internal
        // polygons, not node DISC fills — the surface this law protects. The
        // claim mark is a ring badge with a canvas-bg keyline and never sits
        // on the material, so the facet palette is out of the law's scope.
        let facet_suffixes = ["-light", "-mid", "-hi", "-glow", "-glow-dim", "-sheen"];
        for t in COLORS {
            if !t.name.starts_with("resource-") || t.name.ends_with("-dark") {
                continue;
            }
            if facet_suffixes.iter().any(|s| t.name.ends_with(s)) {
                continue;
            }
            if legacy.contains(&t.name) {
                continue;
            }
            for mark in marks {
                let ratio = contrast(t.value, color(mark));
                assert!(
                    ratio >= 1.5,
                    "{} ({}) vs {} ({}) contrast {ratio:.2}:1 < 1.5:1 — the fill would camouflage the mark",
                    t.name,
                    t.value,
                    mark,
                    color(mark),
                );
            }
        }
    }
}
