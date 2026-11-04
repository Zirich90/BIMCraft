//! Storm drains sized as NBR 10844 (Instalações prediais de águas pluviais, 1989) asks. Pure functions of their input: the
//! `storm_size` command is their caller, so none of this needs a pipe network in the model.
//!
//! - The design flow is `Q = I * A / 60` (L/min, item 5.3.1): `I` the rainfall intensity in mm/h (item 5.1: by place and return
//!   period; item 5.1.4: "para construção até 100 m² de área de projeção horizontal, salvo casos especiais, pode-se adotar I = 150 mm/h") and `A` the area that drains, in m², as the standard
//!   counts it.
//! - A downpipe ("condutor vertical") takes the inner diameter its Figure 3 abacuses give: (a) a gutter with a sharp-edged
//!   outlet, (b) with a funnel outlet, by `Q`, the water depth in the gutter `H` (mm) and the downpipe's length `L` (m): the
//!   higher of what the `H` curves and the `L` curves give at `Q`, interpolating between curves; at least 70 mm inside.
//! - A horizontal drain ("condutor horizontal") takes Manning-Strickler with the water at two thirds of the inner diameter,
//!   `Q = 60000 A R^(2/3) i^(1/2) / n` (L/min), `n` 0.011 for PVC, which the standard's Table 2 gives (Table 4 is the flows computed
//!   with it); PVC is the only material here; at least 0.5 % slope.
//!
//! Source: NBR 10844:1989 itself (a paid ABNT standard), read from a licensed copy: item 5.6.4 (the method: raise a vertical at Q to
//! the H and L curves, interpolate between existing curves when there is none, take the highest intersection, carry it to the D
//! axis), 5.6.3 (70 mm), 5.7.1 (0.5 %), 5.7.2 (two thirds full) and Figure 3, "Ábacos para a determinação de diâmetros de condutores
//! verticais", (a) gutter with a sharp-edged outlet, (b) with a funnel outlet, on page 8 of the copy (the page of 5.6.4 is 7). The
//! abacuses were built for rough pipes (f = 0.04) with two bends at the base (5.6.4.2). Table 4 (Manning, n 0.011) is a formula and
//! is computed, not stored; only its 32 values are in the tests, as a check. Figure 3 is a drawing, not a table, so the fits are this
//! project's own: each curve was digitised from the figure, its own grid calibrating the reading. The standard adopts the nominal
//! diameter whose inside diameter is at least the value read; the command replies the inside diameter read, the nominal is the
//! caller's choice. Each `H` curve is a straight line `D = m Q + b` (mm, L/min) and each `L` curve `ln Q = c0 + c1 ln D + c2 (ln D)^2`, to
//! within 0.2 to 0.6 mm (rms) of the drawn lines. Off the chart (H under 50 or over 100 mm, L under 0.3 m, D over 150 mm)
//! nothing is read, and the caller says so.

/// The smallest inner diameter of a downpipe, mm.
pub const MIN_DOWNPIPE_MM: f64 = 70.0;
/// The smallest slope of a horizontal drain, as a fraction.
pub const MIN_SLOPE: f64 = 0.005;
/// Manning's roughness of PVC.
pub const N_PVC: f64 = 0.011;
/// The rainfall intensity the standard allows for up to 100 m² of roof in plan, mm/h.
pub const DEFAULT_RAIN: f64 = 150.0;
/// The largest roof in plan, m², for which that default may be used; above it the caller gives the rainfall.
pub const DEFAULT_RAIN_MAX_AREA: f64 = 100.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outlet {
    /// A gutter with a sharp-edged outlet: Figure 3 (a).
    Edge,
    /// A gutter with a funnel outlet: Figure 3 (b).
    Funnel,
}

impl Outlet {
    pub fn parse(s: &str) -> Option<Outlet> {
        match s {
            "edge" => Some(Outlet::Edge),
            "funnel" => Some(Outlet::Funnel),
            _ => None,
        }
    }
}

/// One abacus: the `H` lines (depth mm, slope m, intercept b: `D = m Q + b`) and the `L` curves (length m, c0, c1, c2).
struct Abacus {
    h: &'static [(f64, f64, f64)],
    l: &'static [(f64, [f64; 3])],
}

const EDGE: Abacus = Abacus {
    h: &[
        (50.0, 0.225112, -35.9298),
        (60.0, 0.180246, -46.8196),
        (70.0, 0.141298, -52.2541),
        (80.0, 0.117392, -61.4213),
        (90.0, 0.0970961, -66.4115),
        (100.0, 0.0831159, -74.0949),
    ],
    l: &[
        (0.3, [-5.335566, 2.955357, -0.0699641]),
        (0.6, [-2.474349, 1.861605, 0.03953277]),
        (1.0, [-3.238297, 2.271285, -0.006142782]),
        (1.5, [-3.909381, 2.662537, -0.0534521]),
        (2.0, [-1.863548, 1.785209, 0.04467243]),
        (3.0, [-2.737448, 2.226814, -0.004120258]),
        (6.0, [-1.68533, 1.777841, 0.05398324]),
        (25.0, [-2.783537, 2.265283, 0.01298375]),
        (f64::INFINITY, [-3.356874, 2.528462, -0.01051506]),
    ],
};

const FUNNEL: Abacus = Abacus {
    h: &[
        (50.0, 0.185536, -31.2141),
        (60.0, 0.148011, -43.985),
        (70.0, 0.115612, -45.8972),
        (80.0, 0.0924092, -50.0707),
        (90.0, 0.0777569, -56.8891),
        (100.0, 0.0672002, -64.1189),
    ],
    l: &[
        (0.3, [-4.579124, 2.661122, -0.0352465]),
        (0.6, [-0.4543047, 1.02079, 0.1325171]),
        (1.0, [-2.228662, 1.905355, 0.03019043]),
        (1.5, [2.299307, -0.005009665, 0.2364149]),
        (2.0, [-5.589389, 3.483486, -0.1450473]),
        (3.0, [0.03895126, 0.9977121, 0.1351049]),
        (6.0, [-1.222848, 1.558281, 0.08180418]),
        (25.0, [4.10884, -0.9274304, 0.3834927]),
        (f64::INFINITY, [-5.859644, 3.620164, -0.1288296]),
    ],
};

/// What a downpipe's abacus reading is off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Off {
    /// The water depth in the gutter is under 50 or over 100 mm.
    Depth,
    /// The downpipe is shorter than 0.3 m.
    Length,
    /// It needs more than 150 mm inside.
    Diameter,
}

/// A downpipe's inner diameter in mm (at least 70), and what the `H` and `L` curves each gave.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Downpipe {
    pub diameter: f64,
    pub by_depth: f64,
    pub by_length: f64,
}

fn ln_q(c: &[f64; 3], d: f64) -> f64 {
    c[0] + c[1] * d.ln() + c[2] * d.ln().powi(2)
}

/// `D` (mm) on an `L` curve at `Q`: 50 when the curve is above `Q` at the chart's foot, infinite beyond its top.
fn d_on_curve(c: &[f64; 3], q: f64) -> f64 {
    let t = q.ln();
    if ln_q(c, 50.0) >= t {
        return 50.0;
    }
    if ln_q(c, 150.0) < t {
        return f64::INFINITY;
    }
    let (mut lo, mut hi) = (50.0, 150.0);
    for _ in 0..60 {
        let mid = (lo + hi) / 2.0;
        if ln_q(c, mid) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) / 2.0
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// The inner diameter a downpipe needs, as Figure 3 gives it, for the flow `q` (L/min), the water depth `h` (mm) and the length `l` (m).
pub fn downpipe_diameter(outlet: Outlet, q: f64, h: f64, l: f64) -> Result<Downpipe, Off> {
    let a = if outlet == Outlet::Funnel { &FUNNEL } else { &EDGE };
    if h.is_nan() || !(50.0 - 1e-9..=100.0 + 1e-9).contains(&h) {
        return Err(Off::Depth);
    }
    if l.is_nan() || l < 0.3 - 1e-9 {
        return Err(Off::Length);
    }
    if q.is_nan() || q <= 0.0 {
        return Ok(Downpipe { diameter: MIN_DOWNPIPE_MM, by_depth: 0.0, by_length: 0.0 });
    }
    // the H curves: straight lines, linear between them
    let at = |row: &(f64, f64, f64)| row.1 * q + row.2;
    let hi = a.h.iter().position(|r| r.0 >= h - 1e-9).unwrap_or(a.h.len() - 1);
    let (h1, h0) = (a.h[hi], a.h[hi.saturating_sub(1)]);
    let by_depth = if h1.0 == h || hi == 0 { at(&h1) } else { lerp(at(&h0), at(&h1), (h - h0.0) / (h1.0 - h0.0)) };
    // the L curves, each never above the one of the shorter downpipe; linear in L between them (in 1/L between 25 m and infinity)
    let mut ds: Vec<f64> = Vec::with_capacity(a.l.len());
    for c in a.l {
        let d = d_on_curve(&c.1, q);
        ds.push(ds.last().map_or(d, |prev| d.min(*prev)));
    }
    let j = a.l.iter().position(|c| c.0 >= l - 1e-9).unwrap_or(a.l.len() - 1);
    let (c1, c0) = (a.l[j].0, a.l[j.saturating_sub(1)].0);
    let by_length = if c1 == l || j == 0 {
        ds[j]
    } else if c1.is_infinite() {
        lerp(ds[j - 1], ds[j], (1.0 / c0 - 1.0 / l) / (1.0 / c0))
    } else {
        lerp(ds[j - 1], ds[j], (l - c0) / (c1 - c0))
    };
    // a curve off the chart between two of the L curves gives NaN, which `max` would drop: that is an error, never a smaller pipe
    if by_depth.is_nan() || by_length.is_nan() {
        return Err(Off::Diameter);
    }
    let d = by_depth.max(by_length);
    if !d.is_finite() || d > 150.0 + 1e-9 {
        return Err(Off::Diameter);
    }
    Ok(Downpipe { diameter: d.max(MIN_DOWNPIPE_MM), by_depth, by_length })
}

/// Manning-Strickler at two thirds full: L/min for an inner diameter (mm), a slope (fraction) and a roughness `n`.
pub fn manning_flow(inside: f64, slope: f64, n: f64) -> f64 {
    if slope.is_nan() || slope <= 0.0 || inside.is_nan() || inside <= 0.0 || n.is_nan() || n <= 0.0 {
        return 0.0;
    }
    let d = inside / 1000.0;
    let theta = 2.0 * (1.0 - 2.0 * (2.0 / 3.0f64)).acos();
    let area = d * d / 8.0 * (theta - theta.sin());
    let perimeter = d * theta / 2.0;
    60000.0 / n * area * (area / perimeter).powf(2.0 / 3.0) * slope.sqrt()
}

/// The inner diameter (mm) at which a horizontal drain carries `q` L/min at two thirds full: the flow grows as the diameter to the 8/3.
pub fn horizontal_diameter(q: f64, slope: f64, n: f64) -> f64 {
    let one = manning_flow(100.0, slope, n);
    if one <= 0.0 { f64::NAN } else { 100.0 * (q / one).powf(3.0 / 8.0) }
}

/// `Q = I * A / 60`, L/min, for `i` mm/h over `a` m².
pub fn design_flow(i: f64, a: f64) -> f64 {
    if i > 0.0 && a > 0.0 { i * a / 60.0 } else { 0.0 }
}

#[cfg(test)]
mod tests;
