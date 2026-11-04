//! Hydrants and hose reels ("hidrantes e mangotinhos") sized as Instrução Técnica 22/2025 of the Corpo de Bombeiros of São Paulo
//! (CBPMESP, updated by Portaria CCB 003/800/25) asks. Pure functions of their input: the `hydrant_size` command is their caller,
//! so none of this needs a pipe network in the model. The numbers are the IT's own tables:
//!
//! - Tabela 2: the system types (1 to 5): nozzle and hose DN, the hose's length, the flow of one jet (L/min) and the head it needs
//!   at the hydrant's nozzle (mca). Type 4 comes with a DN 40 or a DN 65 nozzle.
//! - Tabela 3: the type of system and the fire reserve (RTI, m³) by the occupation's column (1 to 5) and the built area's band.
//! - Tabela 1: Hazen-Williams' factor C by the pipe's material (new pipes); the head loss is item 4.8.10.2's
//!   `J = 605 Q^1.85 C^-1.85 D^-4.87 10^4` (m/m, Q in L/min, D the inner diameter in mm).
//! - Item 4.8.3: the design flow is that of the two most unfavourable jets; 4.8.12 limits the speed to 5 m/s (4.8.11 is the pump's
//!   suction pipe, 2 m/s with negative suction and 3 m/s with positive, and is not checked here); 4.8.8 the
//!   head at a nozzle to 100 mca; 4.11.6 the smallest pipe to DN 65 (DN 50 for types 1 and 2).
//!
//! This is the rule of São Paulo only (IT 22/2025 of the CBPMESP); no other state's. The band edges of Tabela 3 are taken as
//! inclusive above (`area <= 2500` is the first band): the IT's rows read "Até 2.500 m²", "Acima de 2.500 até 5.000 m²" and so on. Its
//! notes 1 and 2 (a system of one type lower where the occupation has, or has not, sprinklers) are not applied. What `need_mca` covers is
//! the nozzle head, the straight pipe's loss and the rise: no hose or fitting loss, and it checks no pressure at the source.

/// One system type of Tabela 2: (key, type, nozzle DN mm, hose DN mm, hose length m, one jet L/min, head mca).
pub const TYPES: &[(&str, u8, f64, f64, f64, f64, f64)] = &[
    ("1", 1, 25.0, 25.0, 30.0, 100.0, 80.0),
    ("2", 2, 40.0, 40.0, 30.0, 150.0, 30.0),
    ("3", 3, 40.0, 40.0, 30.0, 200.0, 40.0),
    ("4-40", 4, 40.0, 40.0, 30.0, 300.0, 65.0),
    ("4-65", 4, 65.0, 65.0, 30.0, 300.0, 30.0),
    ("5", 5, 65.0, 65.0, 30.0, 600.0, 60.0),
];

/// The keys of `TYPES`.
pub const TYPE_KEYS: &[&str] = &["1", "2", "3", "4-40", "4-65", "5"];

/// Upper limits of Tabela 3's area bands (m²); the last band is above the last limit.
pub const AREA_BANDS: &[f64] = &[2500.0, 5000.0, 10000.0, 20000.0, 50000.0];

/// Tabela 3, one list per column (1 to 5), one (type, reserve in m³) per band, smallest area first.
const RTI: [[(u8, f64); 6]; 5] = [
    [(1, 5.0), (1, 8.0), (1, 12.0), (1, 18.0), (1, 25.0), (1, 35.0)],
    [(2, 8.0), (2, 12.0), (2, 18.0), (2, 25.0), (2, 35.0), (2, 48.0)],
    [(3, 12.0), (3, 18.0), (3, 25.0), (3, 35.0), (3, 48.0), (3, 70.0)],
    [(4, 28.0), (4, 32.0), (4, 48.0), (4, 64.0), (4, 96.0), (4, 120.0)],
    [(4, 32.0), (4, 48.0), (5, 64.0), (5, 96.0), (5, 120.0), (5, 180.0)],
];

/// Tabela 1: Hazen-Williams' C by material, new pipes. `steel` is black steel in a dry system, C 100, the lowest of the steel
/// rows, so a head loss is never underrated; `steel-wet` is galvanised steel, or black steel in a wet system, C 120.
pub const MATERIALS: &[(&str, f64)] = &[("steel", 100.0), ("steel-wet", 120.0), ("copper", 150.0), ("pvc", 150.0), ("pex", 150.0)];

/// The fastest water may go, m/s (item 4.8.12).
pub const MAX_SPEED: f64 = 5.0;
/// The most head allowed at a nozzle, mca (item 4.8.8).
pub const MAX_NOZZLE_HEAD: f64 = 100.0;

/// A system type (1 to 5) and the fire reserve (m³) of a column of Tabela 3 for a built area (m²), or `None` for a column or an
/// area the table does not have.
pub fn system_for(column: u8, area: f64) -> Option<(u8, f64)> {
    let list = RTI.get(usize::from(column).checked_sub(1)?)?;
    if area.is_nan() || area <= 0.0 {
        return None;
    }
    let band = AREA_BANDS.iter().position(|u| area <= *u).unwrap_or(AREA_BANDS.len());
    list.get(band).copied()
}

/// A row of Tabela 2 by key.
pub fn kind(key: &str) -> Option<&'static (&'static str, u8, f64, f64, f64, f64, f64)> {
    TYPES.iter().find(|t| t.0 == key)
}

/// Hazen-Williams' C for a material.
pub fn hazen_c(material: &str) -> Option<f64> {
    MATERIALS.iter().find(|m| m.0 == material).map(|m| m.1)
}

/// `J` (m of water per m) for `q` L/min in an inner diameter `d` mm.
pub fn hazen_j(q: f64, c: f64, d: f64) -> f64 {
    605.0 * q.powf(1.85) * c.powf(-1.85) * d.powf(-4.87) * 1e4
}

/// Speed (m/s) for `q` L/min in an inner diameter `d` mm.
pub fn speed(q: f64, d: f64) -> f64 {
    (q / 60000.0) / (std::f64::consts::PI * (d / 1000.0).powi(2) / 4.0)
}

/// The design flow (L/min): the two most unfavourable jets of the type, so its flow times the hydrants, two at most.
pub fn design_flow(key: &str, hydrants: u32) -> f64 {
    kind(key).map_or(0.0, |t| t.5 * f64::from(hydrants.min(2)))
}

/// The smallest nominal diameter (mm) a hydrant pipe may have: DN 50 for types 1 and 2, DN 65 for the others.
pub fn min_nominal(key: &str) -> f64 {
    if kind(key).is_some_and(|t| t.1 <= 2) { 50.0 } else { 65.0 }
}

#[cfg(test)]
mod tests;
