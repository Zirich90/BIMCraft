//! Pattern geometry shared by the drawing exporters and the creator's live preview.
use super::{LinePattern, Segment};
use crate::{
    geom::P2,
    render::{Prim, Rgb},
    text::{self, Face, Source},
};

// Bound both work and output for hostile files or extreme zoom. Tiny patterns become solid.
const MAX_STEPS: usize = 8192;
const MAX_POINTS: usize = 131_072;

struct Path {
    pts: Vec<P2>,
    along: Vec<f64>,
    total: f64,
}
impl Path {
    fn new(pts: &[(f32, f32)]) -> Self {
        let pts: Vec<P2> = pts.iter().filter(|(x, y)| x.is_finite() && y.is_finite()).map(|&(x, y)| P2(x as f64, y as f64)).collect();
        let mut along = vec![0.0];
        for w in pts.windows(2) {
            along.push(along.last().copied().unwrap_or(0.0) + w[0].dist(w[1]));
        }
        let total = along.last().copied().unwrap_or(0.0);
        Self { pts, along, total }
    }
    fn at(&self, s: f64) -> (P2, P2) {
        let s = s.clamp(0.0, self.total);
        let i = self.along.partition_point(|v| *v < s).max(1).min(self.pts.len() - 1);
        let (a, b) = (self.pts[i - 1], self.pts[i]);
        let u = (b - a).unit();
        (a + u * (s - self.along[i - 1]), u)
    }
    fn offset(&self, s: f64, by: f64) -> (f32, f32) {
        let (p, u) = self.at(s);
        let i = self.along.partition_point(|v| *v < s);
        // At a corner, intersect the two parallel offsets. This keeps each side parallel instead of cutting diagonally
        // back toward the next vertex; a bounded miter avoids spikes at a reversal.
        let miter = |a: P2, b: P2| {
            let denominator = 1.0 + a.dot(b);
            if denominator > 0.1 { (a.perp() + b.perp()) * (1.0 / denominator) } else { a.perp() }
        };
        let closed = self.pts.first().zip(self.pts.last()).is_some_and(|(a, b)| a.dist(*b) < 1e-7);
        let normal = if closed && (s.abs() < 1e-7 || (s - self.total).abs() < 1e-7) {
            let n = self.pts.len();
            miter((self.pts[n - 1] - self.pts[n - 2]).unit(), (self.pts[1] - self.pts[0]).unit())
        } else if i > 0 && i + 1 < self.pts.len() && (self.along[i] - s).abs() < 1e-7 {
            miter(u, (self.pts[i + 1] - self.pts[i]).unit())
        } else {
            u.perp()
        };
        let p = p + normal * by;
        (p.0 as f32, p.1 as f32)
    }
    fn span(&self, a: f64, b: f64, by: f64) -> Vec<(f32, f32)> {
        let mut out = vec![self.offset(a, by)];
        let first = self.along.partition_point(|s| *s <= a);
        let last = self.along.partition_point(|s| *s < b);
        for &s in &self.along[first..last] {
            out.push(self.offset(s, by));
        }
        out.push(self.offset(b, by));
        out
    }
}

/// A graphic and whether the line pen sizes it (including a dot's filled disk).
pub(crate) struct Graphic {
    pub prim: Prim,
    pub weighted: bool,
}

impl LinePattern {
    /// Draw along a pixel polyline, preserving phase through corners and curves. The font is bundled IBM Plex Sans on
    /// every platform. Each glyph rotates with its own tangent; text is centered across and along its reserved segment.
    /// If it is longer than that segment it scales uniformly to fit, with `size` as its maximum capital height.
    pub fn draw(&self, pts: &[(f32, f32)], px_per_mm: f32, width: f32, color: Rgb) -> Vec<Prim> {
        self.geometry(pts, px_per_mm, width, color).into_iter().map(|g| g.prim).collect()
    }

    pub(crate) fn geometry(&self, pts: &[(f32, f32)], px_per_mm: f32, width: f32, color: Rgb) -> Vec<Graphic> {
        if pts.len() < 2 || !px_per_mm.is_finite() || px_per_mm <= 0.0 {
            return Vec::new();
        }
        if self.0.iter().all(|s| matches!(s, Segment::Space(_))) {
            return Vec::new();
        }
        let path = Path::new(pts);
        if path.pts.len() < 2 || path.total <= 1e-9 {
            return Vec::new();
        }
        let line = |pts| Prim::Line { pts, width, color, dash: None, closed: false };
        let k = px_per_mm as f64;
        let repeat = self.0.iter().map(Segment::length).sum::<f64>() * k;
        if self.check("preview").is_err() || repeat < 2.0 || path.total / repeat * self.0.len() as f64 > MAX_STEPS as f64 {
            return vec![Graphic { prim: line(pts.to_vec()), weighted: true }];
        }
        let mut out = Vec::new();
        let mut used = 0;
        let mut start = 0.0;
        for segment in self.0.iter().cycle().take(MAX_STEPS) {
            if start >= path.total {
                break;
            }
            let length = segment.length() * k;
            let end = (start + length).min(path.total);
            let mut made = Vec::new();
            match segment {
                Segment::Space(_) => {}
                Segment::Dash(_) => made.push(line(path.span(start, end, 0.0))),
                Segment::Dot | Segment::DotLength(_) => {
                    let s = start + length * 0.5;
                    if s <= path.total {
                        let (p, _) = path.at(s);
                        let ring = (0..24)
                            .map(|i| {
                                let t = std::f64::consts::TAU * i as f64 / 24.0;
                                (
                                    (p.0 + t.cos() * if width > 0.0 { width as f64 * 0.5 } else { 0.5 }) as f32,
                                    (p.1 + t.sin() * if width > 0.0 { width as f64 * 0.5 } else { 0.5 }) as f32,
                                )
                            })
                            .collect();
                        made.push(Prim::Fill { rings: vec![ring], color });
                    }
                }
                Segment::Offset { count, distance, .. } => {
                    for i in 0..*count {
                        let by = (i as f64 - (*count as f64 - 1.0) / 2.0) * distance * k;
                        made.push(line(path.span(start, end, by)));
                    }
                }
                Segment::Wave { frequency, amplitude, .. } | Segment::Zigzag { frequency, amplitude, .. } => {
                    let zig = matches!(segment, Segment::Zigzag { .. });
                    // Zigzag's quarter-cycle vertices include every peak exactly, even when the last cycle is partial.
                    let intervals = if zig { 4.0 } else { 32.0 };
                    let count = ((end - start) / length * frequency * intervals).ceil().max(1.0) as usize;
                    let mut run = Vec::new();
                    for i in 0..=count {
                        let d = (i as f64 * length / (frequency * intervals)).min(end - start);
                        let t = d / length * frequency;
                        let y = if zig {
                            2.0 / std::f64::consts::PI * (std::f64::consts::TAU * t).sin().asin()
                        } else {
                            (std::f64::consts::TAU * t).sin()
                        };
                        run.push(path.offset(start + d, y * amplitude * k));
                    }
                    made.push(line(run));
                }
                Segment::Text { size, text: label, .. } => {
                    let face = Face::bundled(Source::Sans, false, false);
                    let shapes: Vec<_> = label.chars().map(|c| text::shape(&face, c)).collect();
                    let natural: f64 = shapes.iter().map(|s| s.advance as f64).sum();
                    let size = (size * k).min(length / natural.max(1e-9));
                    let mut x = start + (length - natural * size) / 2.0;
                    for shape in shapes {
                        let advance = shape.advance as f64 * size;
                        if x + advance > path.total {
                            break;
                        }
                        let mid = x + advance * 0.5;
                        let (p, u) = path.at(mid);
                        let at = |&(gx, gy): &(f32, f32)| {
                            // Pixel y points down. Cap-height midpoint sits on the centerline.
                            let p = p + u * (gx as f64 * size - advance * 0.5) + u.perp() * ((0.5 - gy as f64) * size);
                            (p.0 as f32, p.1 as f32)
                        };
                        if !shape.fill.is_empty() {
                            made.push(Prim::Fill { rings: shape.fill.iter().map(|r| r.iter().map(at).collect()).collect(), color });
                        }
                        for stroke in &shape.strokes {
                            made.push(Prim::Line {
                                pts: stroke.iter().map(at).collect(),
                                width: shape.stroke * size as f32,
                                color,
                                dash: None,
                                closed: false,
                            });
                        }
                        x += advance;
                    }
                }
            }
            used += made
                .iter()
                .map(|p| match p {
                    Prim::Line { pts, .. } => pts.len(),
                    Prim::Fill { rings, .. } => rings.iter().map(Vec::len).sum(),
                    _ => 0,
                })
                .sum::<usize>();
            if used > MAX_POINTS {
                return vec![Graphic { prim: line(pts.to_vec()), weighted: true }];
            }
            let weighted = !matches!(segment, Segment::Text { .. });
            out.extend(made.into_iter().map(|prim| Graphic { prim, weighted }));
            start += length;
        }
        out
    }
}
