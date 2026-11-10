//! The fill of a conduit as NBR 5410:2004 asks: the sum of the cross-sections of the conductors it holds (phases, neutral and
//! protective, by their outside diameter, insulation included) over the conduit's inside cross-section, at most 53 % for one
//! cable, 31 % for two and 40 % for three or more. The physical data are the makers' own, in [`tables`], each family naming
//! its maker, document, page and address; the criterion is the standard's. A conduit's inside diameter is not printed in those
//! catalogues, so it is derived: the outside diameter less twice the wall. A cable the catalogue has no diameter for (EPR or
//! XLPE insulation, sections above 240 mm²) is not guessed: the fill is pending. Cable trays are not sized here.

mod tables;

use crate::nbr5410::{Phases, neutral_section, protective_section};
pub use tables::{CABLE_OD, FAMILIES, Family};

/// A conduit size: its mark, DN in mm, outside diameter in mm and wall in mm.
pub type Size = (&'static str, f64, f64, f64);

/// The most a conduit may be filled, as a fraction, by the number of cables in it.
pub fn limit(cables: usize) -> f64 {
    match cables {
        0 | 1 => 0.53,
        2 => 0.31,
        _ => 0.40,
    }
}

/// The conductors a circuit puts in its conduit, as the section of each: the phases, the neutral and the protective conductor.
pub fn conductors(phases: Phases, section: f64, reduced_neutral: bool) -> Vec<f64> {
    let mut out = vec![section; phases.count()];
    match phases {
        Phases::PhaseNeutral => out.push(section),
        Phases::ThreeNeutral => out.push(neutral_section(section, reduced_neutral)),
        _ => {}
    }
    out.push(protective_section(section));
    out
}

pub fn family(key: &str) -> Option<&'static Family> {
    FAMILIES.iter().find(|f| f.key == key)
}

/// A size's inside diameter in mm: its outside diameter less twice its wall.
pub fn inside(size: &Size) -> f64 {
    size.2 - 2.0 * size.3
}

/// The outside diameter of a flexible PVC cable of `section` mm², when the catalogue has it.
pub fn cable_diameter(section: f64) -> Option<f64> {
    CABLE_OD.iter().find(|c| (c.0 - section).abs() < 1e-9).map(|c| c.1)
}

/// A conduit size filled by conductors of the given sections.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fill {
    pub cables: usize,
    /// The cables' cross-section and the conduit's inside, mm².
    pub area: f64,
    pub inside: f64,
    /// area / inside, and the most the standard allows.
    pub ratio: f64,
    pub limit: f64,
    pub ok: bool,
}

/// The fill of `size` by `sections`; none when a cable has no diameter in the catalogue.
pub fn fill(size: &Size, sections: &[f64]) -> Option<Fill> {
    let mut area = 0.0;
    for s in sections {
        let d = cable_diameter(*s)?;
        area += std::f64::consts::PI * d * d / 4.0;
    }
    let d = inside(size);
    let inside = std::f64::consts::PI * d * d / 4.0;
    let (ratio, limit) = (area / inside, limit(sections.len()));
    Some(Fill { cables: sections.len(), area, inside, ratio, limit, ok: ratio <= limit + 1e-9 })
}

/// The smallest size of a family that takes `sections`, when there is one.
pub fn smallest(family: &Family, sections: &[f64]) -> Option<&'static Size> {
    family.sizes.iter().find(|s| fill(s, sections).is_some_and(|f| f.ok))
}

#[cfg(test)]
mod tests;
