//! Asset provider abstraction (SDD §7): Install / CommunityPack / Placeholder
//! behind one trait, chosen at onboarding, swappable in settings. The UI never
//! blocks on icons — the diagonal-stripe placeholder chip is a first-class
//! rendering, not an error state.
//!
//! v1 ships `PlaceholderAssets` only: the pinned community icon pack shares
//! the SAME unresolved licensing question as the map tiles (see DECISIONS.md);
//! bundling is blocked on that human decision, not on this interface.

pub trait AssetSource {
    /// PNG bytes for an item/machine class icon, if this source has one.
    fn icon(&self, class: &str) -> Option<Vec<u8>>;
    /// Human-readable label for the settings surface.
    fn label(&self) -> &'static str;
}

/// The honest default: no icons, placeholder grammar everywhere.
pub struct PlaceholderAssets;

impl AssetSource for PlaceholderAssets {
    fn icon(&self, _class: &str) -> Option<Vec<u8>> {
        None
    }
    fn label(&self) -> &'static str {
        "PLACEHOLDER — icon pack pending licensing review"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_never_yields_icons() {
        let src = PlaceholderAssets;
        assert!(src.icon("Desc_IronRod_C").is_none());
        assert!(src.label().contains("PLACEHOLDER"));
    }
}
