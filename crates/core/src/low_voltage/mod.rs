//! Low-voltage design and document production.
pub(crate) mod calc;
pub(crate) mod carrier;
pub(crate) mod catalogue;
pub(crate) mod diagram_devices;
pub(crate) mod diagram_layout;
pub(crate) mod diagrams;
pub(crate) mod documents;
pub(crate) mod faults;
pub(crate) mod fire_design;
pub(crate) mod lines;
pub(crate) mod ods;
mod odt;
#[cfg(test)]
mod odt_tests;
pub mod systems;
pub(crate) mod wire_geometry;
pub(crate) mod zone_geometry;
pub(crate) mod zones;

pub use lines::{route_ending_at, route_head, route_members};

use crate::model::{Element, Model};

pub(crate) fn check(model: &Model) -> Result<(), String> {
    lines::check(model)
}

pub(crate) fn check_element(model: &Model, who: &str, element: &Element) -> Result<(), String> {
    match element {
        Element::FireZone(zone) => zones::check_zone(model, who, zone),
        Element::ElectricalTerminal(terminal) => lines::check_terminal(model, who, terminal),
        _ => Ok(()),
    }
}

// Add fork-specific migrations here only when older data needs conversion.
// Existing missing fields are handled by serde defaults.
pub(crate) fn migrate(_model: &mut Model) {}

pub(crate) fn unknown_fields(category: &str, value: &serde_json::Value) -> Option<Vec<String>> {
    use crate::lenient::unknown;
    Some(match category {
        "electricalline" => unknown::<lines::ElectricalLine>(value),
        "electricalterminal" => unknown::<lines::ElectricalTerminal>(value),
        "fire_zone" => unknown::<zones::FireZone>(value),
        _ => return None,
    })
}
