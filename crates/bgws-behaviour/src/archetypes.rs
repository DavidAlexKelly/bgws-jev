//! Which built-in template a unit uses when nothing else is assigned, by what
//! it is (RUST_ENGINE.md §10.1; DECISION_FLOW_GRAPHS.md §4.2). A port of
//! `behaviour/archetypes.ts`, shared by the engine and the store so both
//! always agree on a unit's default.

/// The land default: decision points D0–D12.
pub const DEFAULT_TEMPLATE_ID: &str = "core/default";

/// The world default: every world system's decision points.
pub const WORLD_DEFAULT_TEMPLATE: &str = "core/world";

/// The default template for a unit or asset of this target class.
pub fn default_template_for_class(target_class: Option<&str>) -> &'static str {
    match target_class {
        Some("armoured_vehicle") => "core/armour",
        Some("foot") => "core/infantry",
        _ => DEFAULT_TEMPLATE_ID,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_by_class() {
        assert_eq!(default_template_for_class(Some("armoured_vehicle")), "core/armour");
        assert_eq!(default_template_for_class(Some("foot")), "core/infantry");
        assert_eq!(default_template_for_class(Some("soft_skin")), DEFAULT_TEMPLATE_ID);
        assert_eq!(default_template_for_class(None), DEFAULT_TEMPLATE_ID);
    }
}
