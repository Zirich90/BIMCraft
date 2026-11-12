//! Detector bays and mounting surfaces under ceiling beams (SP 484 Amendment 1, 6.6.36 and 6.6.38).

use crate::geom::{self, P2, Region};
use crate::model::{Element, Model, TOL};

struct Band {
    lo: f64,
    hi: f64,
    depth: f64,
    along: bool,
    ends: Option<(f64, f64)>,
    footprint: Vec<P2>,
}

impl Band {
    fn coordinate(&self, at: P2) -> f64 {
        if self.along { at.0 } else { at.1 }
    }

    fn longitudinal(&self, at: P2) -> f64 {
        if self.along { at.1 } else { at.0 }
    }

    fn contains(&self, at: P2) -> bool {
        let cross = self.coordinate(at);
        cross > self.lo && cross < self.hi && self.ends.is_none_or(|(start, end)| (start..end).contains(&self.longitudinal(at)))
    }
}

pub(crate) struct Layout {
    axis: P2,
    stretch: f64,
    bands: Vec<Band>,
    oblique: Vec<(Region, f64)>,
    pub bays: Vec<Region>,
    pub reach: f64,
    crossed: bool,
}

impl Layout {
    fn to_grid(&self, at: P2) -> P2 {
        P2(at.dot(self.axis), at.dot(self.axis.perp()) * self.stretch)
    }

    pub fn to_world(&self, at: P2) -> P2 {
        self.axis * at.0 + self.axis.perp() * (at.1 / self.stretch)
    }

    pub fn clear(&self, at: P2) -> bool {
        if self.oblique.iter().any(|(footprint, depth)| footprint.contains(at) || footprint.edge_distance(at) < (2.0 * depth).min(500.0)) {
            return false;
        }
        let projected = P2(at.dot(self.axis), at.dot(self.axis.perp()));
        self.bands.iter().all(|band| {
            let clearance = (2.0 * band.depth).min(500.0);
            let cross = band.coordinate(projected);
            let normal_gap = (band.lo - cross).max(cross - band.hi).max(0.0);
            let end_gap = band.ends.map_or(0.0, |(start, end)| {
                let longitudinal = band.longitudinal(projected);
                (start - longitudinal).max(longitudinal - end).max(0.0)
            });
            normal_gap.hypot(end_gap) >= clearance
        })
    }

    pub fn on_beam(&self, bay: &Region, end_clearance: f64, remaining: usize) -> Result<Option<(Vec<P2>, f64)>, &'static str> {
        if !self.crossed {
            return Ok(None);
        }
        if !self.oblique.is_empty() {
            return self.oblique_mount(bay, end_clearance, remaining);
        }
        let Some((lo, hi)) = geom::bounds(&bay.outer) else { return Ok(None) };
        let width = (hi.0 - lo.0).min(hi.1 - lo.1);
        let rectangular = rectangle(bay, lo, hi);
        if !rectangular && self.bands.iter().any(|band| width <= 4.0 * band.depth) {
            return Err("mounting under crossed beams requires a rectangular narrow cell");
        }
        let mut faces = self
            .bands
            .iter()
            .filter_map(|band| {
                let (a, b) = (band.coordinate(lo), band.coordinate(hi));
                if (band.hi - a).abs() >= 0.001 && (band.lo - b).abs() >= 0.001 {
                    return None;
                }
                if width > 4.0 * band.depth {
                    return None;
                }
                if band.ends.is_some_and(|(start, end)| end <= band.longitudinal(lo) + TOL || start >= band.longitudinal(hi) - TOL) {
                    return None;
                }
                let middle = (band.lo + band.hi) * 0.5;
                let distance = (middle - a).abs().max((middle - b).abs());
                Some((band, middle, distance))
            })
            .collect::<Vec<_>>();
        if faces.is_empty() {
            return Ok(None);
        }
        faces.sort_by(|a, b| a.2.total_cmp(&b.2));
        for (band, middle, distance) in faces {
            let (a, b) = if band.along { (lo.1, hi.1) } else { (lo.0, hi.0) };
            if band.ends.is_some_and(|(start, end)| a < start - TOL || b > end + TOL) {
                continue;
            }
            if let Ok(row) = row(a, b, self.reach, distance, end_clearance, remaining) {
                let points = row.into_iter().map(|v| if band.along { P2(middle, v) } else { P2(v, middle) }).collect::<Vec<_>>();
                if points.iter().any(|&at| self.bands.iter().any(|other| other.depth > band.depth + TOL && other.contains(at))) {
                    continue;
                }
                return Ok(Some((points.into_iter().map(|at| self.to_world(at)).collect(), band.depth)));
            }
        }
        Err("no beam underside can cover the narrow crossed-beam cell with the required clearance")
    }

    fn deepest(&self) -> f64 {
        self.bands.iter().map(|band| band.depth).chain(self.oblique.iter().map(|(_, depth)| *depth)).fold(0.0, f64::max)
    }

    fn oblique_mount(&self, bay: &Region, margin: f64, remaining: usize) -> Result<Option<(Vec<P2>, f64)>, &'static str> {
        let corners = geom::corners(&bay.outer);
        let width = convex_width(&corners);
        let Some(width) = width.filter(|_| bay.holes.is_empty()) else {
            let clear_width = match bay.holes.as_slice() {
                [] => inset_cell_width(bay),
                holes if holes.iter().map(Vec::len).sum::<usize>() <= 256 => width.map(|width| width.min(holes_clearance(&corners, holes))),
                _ => None,
            };
            if clear_width.is_some_and(|width| width > 4.0 * self.deepest()) {
                return Ok(None);
            }
            return Err("a non-convex cell under oblique beams requires a separately designed mounting surface");
        };
        if width > 4.0 * self.deepest() {
            return Ok(None);
        }
        let corners: Vec<_> = corners.into_iter().map(|at| self.to_world(at)).collect();
        let beams: Vec<_> = self
            .bands
            .iter()
            .map(|band| (band.footprint.as_slice(), band.depth))
            .chain(self.oblique.iter().map(|(footprint, depth)| (footprint.outer.as_slice(), *depth)))
            .collect();
        for &(footprint, depth) in &beams {
            if width > 4.0 * depth {
                continue;
            }
            let center = geom::centroid(footprint);
            for (a, b) in geom::ring_edges(footprint) {
                let axis = (b - a).unit();
                let normal = axis.perp();
                let face = a.dot(normal);
                let middle = center.dot(normal);
                let shared =
                    geom::ring_edges(&corners).find(|(p, q)| (p.dot(normal) - face).abs() < TOL && (q.dot(normal) - face).abs() < TOL);
                let Some((p, q)) = shared else { continue };
                let edge_lo = p.dot(axis).min(q.dot(axis)).max(a.dot(axis));
                let edge_hi = p.dot(axis).max(q.dot(axis)).min(b.dot(axis));
                let (lo, hi) = projected_bounds(&corners, axis);
                let (near, far) = projected_bounds(&corners, normal);
                if (middle < face && near < face - TOL) || (middle > face && far > face + TOL) || (middle - face).abs() <= TOL {
                    continue;
                }
                let across = (middle - near).abs().max((middle - far).abs());
                let Ok(row) = row(lo, hi, self.reach, across, margin, remaining) else { continue };
                if row.iter().any(|&x| x < edge_lo + margin || x > edge_hi - margin) {
                    continue;
                }
                let points: Vec<_> = row.into_iter().map(|x| axis * x + normal * middle).collect();
                // A convex cell is visible through the shared face if every ray to
                // its vertices enters there and the approach misses deeper beams.
                if points.iter().all(|&at| {
                    let hits = corners.iter().map(|&vertex| {
                        let t = (face - middle) / (vertex.dot(normal) - middle);
                        (at + (vertex - at) * t).dot(axis)
                    });
                    let (start, end) = hits.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| (lo.min(x), hi.max(x)));
                    if start < edge_lo - TOL || end > edge_hi + TOL {
                        return false;
                    }
                    let sight = Region { outer: vec![at, axis * start + normal * face, axis * end + normal * face], holes: Vec::new() };
                    !beams.iter().any(|&(other, other_depth)| other_depth > depth + TOL && intersects(&sight, other))
                }) {
                    return Ok(Some((points, depth)));
                }
            }
        }
        Err("no beam underside can cover the narrow oblique cell with the required clearance")
    }

    /// Clause 6.6.36 permits the middle between walls or deep projections when
    /// 500 mm on each side does not fit. Use exact corner coverage for a rectangular bay.
    pub fn centered(
        &self,
        bay: &Region,
        reach: f64,
        end_clearance: f64,
        remaining: usize,
        wall_face: impl Fn(P2, P2) -> bool,
    ) -> Result<Option<Vec<P2>>, &'static str> {
        if self.crossed || self.bands.iter().any(|band| band.ends.is_some()) {
            return Ok(None);
        }
        let Some((lo, hi)) = geom::bounds(&bay.outer) else { return Ok(None) };
        let width = (hi.1 - lo.1) / self.stretch;
        if width >= 1000.0 || !rectangle(bay, lo, hi) {
            return Ok(None);
        }
        let deep_edge = |edge: f64, high_side: bool| {
            self.bands.iter().any(|band| {
                let face = if high_side { band.lo } else { band.hi };
                band.depth >= 250.0 && (face - edge / self.stretch).abs() < 0.001
            })
        };
        let wall = |edge| wall_face(self.to_world(P2(lo.0, edge)), self.to_world(P2(hi.0, edge)));
        if !(deep_edge(lo.1, false) || wall(lo.1)) || !(deep_edge(hi.1, true) || wall(hi.1)) {
            return Ok(None);
        }
        let positions = row(lo.0, hi.0, reach, (hi.1 - lo.1) * 0.5, end_clearance, remaining)?
            .into_iter()
            .map(|x| self.to_world(P2(x, (lo.1 + hi.1) * 0.5)))
            .collect();
        Ok(Some(positions))
    }
}

fn convex_width(corners: &[P2]) -> Option<f64> {
    if !(3..=256).contains(&corners.len()) {
        return None;
    }
    let orientation = geom::area(corners).signum();
    for ((a, b), &c) in geom::ring_edges(corners).zip(corners.iter().cycle().skip(2)) {
        if (b - a).cross(c - b) * orientation < -TOL {
            return None;
        }
    }
    minimum_width(corners)
}

/// Lower bound on each passage around a single hole inside a convex cell.
/// Coverage still uses the original cell, so rays cannot pass through the hole.
fn hole_clearance(corners: &[P2], hole: &[P2]) -> f64 {
    let orientation = geom::area(corners).signum();
    geom::ring_edges(corners).filter(|(a, b)| a.dist(*b) > TOL).fold(f64::INFINITY, |clearance, (a, b)| {
        let inward = (b - a).unit().perp() * orientation;
        let (near, _) = projected_bounds(hole, inward);
        clearance.min(near - a.dot(inward))
    })
}

/// Bound every passage by its clearance to the outer faces and to other holes.
/// The cell's disjoint rings cannot cross, so endpoint-to-edge distances suffice.
fn holes_clearance(corners: &[P2], holes: &[Vec<P2>]) -> f64 {
    let mut clearance = f64::INFINITY;
    for (index, hole) in holes.iter().enumerate() {
        clearance = clearance.min(hole_clearance(corners, hole));
        for other in &holes[..index] {
            for (a, b) in geom::ring_edges(hole) {
                for (c, d) in geom::ring_edges(other) {
                    clearance = clearance.min(geom::dist_to_segment(a, c, d)).min(geom::dist_to_segment(c, a, b));
                }
            }
        }
    }
    clearance
}

/// Each boundary chain between adjacent hull vertices stays outside the core
/// cut at its deepest point. The centroid verifies which side of the boundary
/// contains the remaining core, including when the chain doubles back.
/// Use this bound only for ceiling mounting. Coverage still uses the actual cell.
fn inset_cell_width(bay: &Region) -> Option<f64> {
    let mut corners = geom::corners(&bay.outer);
    if !bay.holes.is_empty() || !(4..=256).contains(&corners.len()) {
        return None;
    }
    if geom::area(&corners) < 0.0 {
        corners.reverse();
    }
    let hull = geom::hull(corners.clone());
    let first = corners.iter().position(|at| Some(at) == hull.first())?;
    corners.rotate_left(first);
    corners.push(*corners.first()?);
    let mut chain_start = 0;
    let mut core = hull.clone();
    for (a, b) in geom::ring_edges(&hull) {
        let chain_end = chain_start + corners.get(chain_start..)?.iter().position(|&at| at == b)?;
        let chain = corners.get(chain_start..=chain_end)?;
        chain_start = chain_end;
        if chain.len() <= 2 {
            continue;
        }
        let axis = (b - a).unit();
        let normal = axis.perp();
        let depth = chain.iter().map(|&at| (at - a).dot(normal)).fold(0.0, f64::max);
        core = geom::clip(&core, |at| (at - a).dot(normal) - depth);
    }
    if core.len() < 3 || geom::area(&core).abs() < TOL || !bay.contains(geom::centroid(&core)) {
        return None;
    }
    minimum_width(&core)
}

fn minimum_width(corners: &[P2]) -> Option<f64> {
    geom::ring_edges(corners)
        .filter(|(a, b)| a.dist(*b) > TOL)
        .map(|(a, b)| {
            let (lo, hi) = projected_bounds(corners, (b - a).unit().perp());
            hi - lo
        })
        .min_by(f64::total_cmp)
}

fn rectangle(bay: &Region, lo: P2, hi: P2) -> bool {
    let at_end = |v: f64, a: f64, b: f64| (v - a).abs() < 0.001 || (v - b).abs() < 0.001;
    let corners = geom::corners(&bay.outer);
    bay.holes.is_empty() && corners.len() == 4 && corners.iter().all(|p| at_end(p.0, lo.0, hi.0) && at_end(p.1, lo.1, hi.1))
}

pub(crate) fn wall_bay(
    region: &Region,
    reach: f64,
    margin: f64,
    remaining: usize,
    wall_face: impl Fn(P2, P2) -> bool,
) -> Result<Option<Vec<P2>>, &'static str> {
    if !region.holes.is_empty() || margin <= 0.0 || margin > 500.0 {
        return Ok(None);
    }
    let corners = geom::corners(&region.outer);
    if corners.len() != 4 {
        return Ok(None);
    }
    let Some(side) = geom::ring_edges(&corners).map(|(a, b)| b - a).max_by(|a, b| a.length().total_cmp(&b.length())) else {
        return Ok(None);
    };
    if side.length() < 2.0 * margin {
        return Ok(None);
    }
    let layout =
        Layout { axis: side.unit(), stretch: 1.0, bands: Vec::new(), oblique: Vec::new(), bays: Vec::new(), reach, crossed: false };
    let bay = Region { outer: corners.into_iter().map(|point| layout.to_grid(point)).collect(), holes: Vec::new() };
    let Some((lo, hi)) = geom::bounds(&bay.outer) else { return Ok(None) };
    if hi.1 - lo.1 >= 2.0 * margin {
        return Ok(None);
    }
    layout.centered(&bay, reach, margin, remaining, wall_face)
}

/// Equal cells along a mounting line. Clamping the first and last centres
/// toward the middle preserves the largest gap between adjacent detectors.
fn row(lo: f64, hi: f64, reach: f64, across: f64, clearance: f64, remaining: usize) -> Result<Vec<f64>, &'static str> {
    let along_reach = ((reach - across) * (reach + across)).sqrt();
    let length = hi - lo;
    if across >= reach || clearance > along_reach || clearance * 2.0 > length {
        return Err("the detector row cannot meet coverage and end clearance");
    }
    let count = (length / (2.0 * along_reach)).ceil().max(1.0);
    if count > remaining as f64 {
        return Err("the detector row exceeds the position limit");
    }
    let pitch = length / count;
    Ok((0..count as usize).map(|i| (lo + pitch * (i as f64 + 0.5)).clamp(lo + clearance, hi - clearance)).collect())
}

pub(crate) fn layout(model: &Model, room: &Region, elevations: (f64, f64), reach: f64, heat: bool) -> Result<Option<Layout>, &'static str> {
    let (floor, ceiling) = elevations;
    let height = ceiling - floor;
    let mut layout =
        Layout { axis: P2(0.0, 0.0), stretch: 1.0, bands: Vec::new(), oblique: Vec::new(), bays: Vec::new(), reach, crossed: false };
    let mut transverse = reach;
    for element in model.elements.values().filter(|element| matches!(element, Element::Beam(_))) {
        let Some(beam) = model.run_geom(element) else { continue };
        let (bottom, top) = beam.z_span();
        if bottom >= ceiling - TOL || top < ceiling - TOL {
            continue;
        }
        let footprint = beam.footprint();
        if !intersects(room, &footprint) {
            continue;
        }
        if bottom <= floor + TOL {
            return Err("a ceiling beam reaches the floor of the detector compartment");
        }
        let direction = P2(beam.b[0] - beam.a[0], beam.b[1] - beam.a[1]).unit();
        if beam.round || beam.roll.abs() > 1e-9 || (beam.a[2] - beam.b[2]).abs() > TOL || direction.length() < 0.5 {
            return Err("automatic detector placement requires horizontal rectangular ceiling beams");
        }
        if layout.bands.is_empty() {
            layout.axis = direction;
        }
        let alignment = layout.axis.dot(direction).abs();
        let along = alignment < 1e-9;
        if layout.bands.len() + layout.oblique.len() >= 128 {
            return Err("too many ceiling beams in one detector compartment");
        }
        if !along && alignment < 1.0 - 1e-9 {
            layout.crossed = true;
            layout.oblique.push((Region { outer: footprint, holes: Vec::new() }, ceiling - bottom));
            continue;
        }
        layout.crossed |= along;
        let direction = if along { layout.axis.perp() } else { layout.axis };
        let (room_start, room_end) = projected_bounds(&room.outer, direction);
        let (start, end) = projected_bounds(&footprint, direction);
        let ends = (start > room_start + TOL || end < room_end - TOL).then_some((start, end));
        let normal = if along { layout.axis } else { layout.axis.perp() };
        let (lo, hi) = projected_bounds(&footprint, normal);
        let depth = ceiling - bottom;
        let (step, wall) = spacing(height, depth, heat);
        transverse = transverse.min(wall).min((step - (hi - lo)) * 0.5);
        layout.bands.push(Band { lo, hi, depth, along, ends, footprint });
    }
    if layout.bands.is_empty() {
        return Ok(None);
    }
    layout.bands.sort_by(|a, b| a.along.cmp(&b.along).then_with(|| a.lo.total_cmp(&b.lo)));
    if layout.bands.windows(2).any(|pair| pair[0].along == pair[1].along && pair[0].hi >= pair[1].lo) {
        return Err("overlapping parallel beams prevent a detector layout");
    }
    let along_bounds = projected_bounds(&room.outer, layout.axis);
    let cross_bounds = projected_bounds(&room.outer, layout.axis.perp());
    if layout.crossed {
        layout.reach = reach.min(crossed_reach(height, layout.deepest(), heat));
    } else {
        if transverse < TOL {
            return Err("wide beams prevent a parallel detector layout");
        }
        layout.stretch = reach / transverse;
    }
    let mut cuts: Vec<_> = layout.oblique.iter().flat_map(|(footprint, _)| geom::ring_edges(&footprint.outer)).collect();
    for band in &layout.bands {
        let point = |fixed, free| {
            let (x, y) = if band.along { (fixed, free) } else { (free, fixed) };
            layout.axis * x + layout.axis.perp() * y
        };
        if let Some((a, b)) = band.ends {
            let footprint = [point(band.lo, a), point(band.lo, b), point(band.hi, b), point(band.hi, a)];
            cuts.extend(geom::ring_edges(&footprint));
        } else {
            let (a, b) = if band.along { cross_bounds } else { along_bounds };
            cuts.extend([band.lo, band.hi].map(|edge| (point(edge, a - TOL), point(edge, b + TOL))));
        }
    }
    let bays = room.partition_by_height(&cuts, |at| {
        if layout.oblique.iter().any(|(footprint, _)| footprint.contains(at)) {
            return None;
        }
        let at = P2(at.dot(layout.axis), at.dot(layout.axis.perp()));
        (!layout.bands.iter().any(|band| band.contains(at))).then_some(0.0)
    })?;
    layout.bays = bays
        .into_iter()
        .filter(|(label, _)| label.is_some())
        .map(|(_, bay)| Region {
            outer: bay.outer.into_iter().map(|at| layout.to_grid(at)).collect(),
            holes: bay.holes.into_iter().map(|hole| hole.into_iter().map(|at| layout.to_grid(at)).collect()).collect(),
        })
        .collect();
    if layout.bays.is_empty() {
        return Err("the ceiling beams occupy the entire detector compartment");
    }
    Ok(Some(layout))
}

fn crossed_reach(height: f64, depth: f64, heat: bool) -> f64 {
    let height = (height / 1000.0).round();
    if depth < height * 100.0 {
        return f64::INFINITY;
    }
    let (smoke, thermal) = if height <= 3.0 {
        (4500.0, 3000.0)
    } else if height == 4.0 {
        (5500.0, 4000.0)
    } else if height == 5.0 {
        (6000.0, 4500.0)
    } else {
        (6600.0, 5000.0)
    };
    if heat { thermal } else { smoke }
}

fn projected_bounds(points: &[P2], axis: P2) -> (f64, f64) {
    points.iter().map(|at| at.dot(axis)).fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), at| (lo.min(at), hi.max(at)))
}

fn intersects(room: &Region, footprint: &[P2]) -> bool {
    let Some(((lo, hi), (a, b))) = geom::bounds(&room.outer).zip(geom::bounds(footprint)) else { return false };
    if a.0 >= hi.0 || a.1 >= hi.1 || b.0 <= lo.0 || b.1 <= lo.1 {
        return false;
    }
    footprint.iter().any(|&at| room.contains(at))
        || room.outer.iter().any(|&at| geom::contains(footprint, at))
        || std::iter::once(&room.outer).chain(&room.holes).flat_map(|ring| geom::ring_edges(ring)).any(|(a, b)| {
            geom::ring_edges(footprint).any(|(c, d)| {
                geom::line_intersection(a, b, c, d)
                    .is_some_and(|at| geom::dist_to_segment(at, a, b) < 1e-3 && geom::dist_to_segment(at, c, d) < 1e-3)
            })
        })
}

fn spacing(height: f64, depth: f64, heat: bool) -> (f64, f64) {
    let height = (height / 1000.0).round();
    if depth < height * 100.0 {
        return if heat { (3800.0, 1900.0) } else { (5000.0, 2500.0) };
    }
    // The table omits exactly 10%; apply the deep-beam limit at equality.
    if heat {
        if height <= 3.0 {
            (1500.0, 1250.0)
        } else if height == 4.0 {
            (2000.0, 1000.0)
        } else if height == 5.0 {
            (2300.0, 1150.0)
        } else {
            (2500.0, 1250.0)
        }
    } else if height <= 3.0 {
        (2300.0, 1150.0)
    } else if height == 4.0 {
        (2800.0, 1400.0)
    } else if height == 5.0 {
        (3000.0, 1500.0)
    } else {
        (3300.0, 1650.0)
    }
}
