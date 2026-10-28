//! Our own single-stroke lettering for plan annotation: capitals, Latin and Cyrillic, digits, a few
//! marks and the accents of Latin languages, drawn on an 8 x 12 grid (y up). Lower case is drawn as capitals,
//! as architectural lettering is. Where a TrueType font is named for it (see [`letters`]), the lettering is
//! set in that font's outlines instead, lower case and all.

use std::sync::OnceLock;
use ttf_parser::{Face, GlyphId, OutlineBuilder};

type Glyph = &'static [&'static [(i8, i8)]];

const O: &[(i8, i8)] = &[(2, 0), (0, 2), (0, 10), (2, 12), (6, 12), (8, 10), (8, 2), (6, 0), (2, 0)];
const P: &[(i8, i8)] = &[(0, 0), (0, 12), (6, 12), (8, 10), (8, 8), (6, 6), (0, 6)];
const QUESTION: Glyph = &[&[(0, 10), (2, 12), (6, 12), (8, 10), (8, 8), (4, 5), (4, 3)], &[(4, 1), (4, 0)]];

fn glyph(c: char) -> Glyph {
    match c {
        'A' => &[&[(0, 0), (4, 12), (8, 0)], &[(1, 3), (7, 3)]],
        'B' => &[&[(0, 0), (0, 12), (6, 12), (8, 10), (8, 8), (6, 6), (0, 6)], &[(6, 6), (8, 4), (8, 2), (6, 0), (0, 0)]],
        'C' => &[&[(8, 10), (6, 12), (2, 12), (0, 10), (0, 2), (2, 0), (6, 0), (8, 2)]],
        'D' => &[&[(0, 0), (0, 12), (5, 12), (8, 9), (8, 3), (5, 0), (0, 0)]],
        'E' => &[&[(8, 12), (0, 12), (0, 0), (8, 0)], &[(0, 6), (6, 6)]],
        'F' => &[&[(8, 12), (0, 12), (0, 0)], &[(0, 6), (6, 6)]],
        'G' => &[&[(8, 10), (6, 12), (2, 12), (0, 10), (0, 2), (2, 0), (6, 0), (8, 2), (8, 5), (5, 5)]],
        'H' => &[&[(0, 0), (0, 12)], &[(8, 0), (8, 12)], &[(0, 6), (8, 6)]],
        'I' => &[&[(2, 12), (6, 12)], &[(4, 12), (4, 0)], &[(2, 0), (6, 0)]],
        'J' => &[&[(8, 12), (8, 2), (6, 0), (2, 0), (0, 2)]],
        'K' => &[&[(0, 0), (0, 12)], &[(8, 12), (0, 4)], &[(3, 7), (8, 0)]],
        'L' => &[&[(0, 12), (0, 0), (8, 0)]],
        'M' => &[&[(0, 0), (0, 12), (4, 6), (8, 12), (8, 0)]],
        'N' => &[&[(0, 0), (0, 12), (8, 0), (8, 12)]],
        'O' => &[O],
        'P' => &[P],
        'Q' => &[O, &[(5, 3), (8, 0)]],
        'R' => &[P, &[(4, 6), (8, 0)]],
        'S' => &[&[(8, 10), (6, 12), (2, 12), (0, 10), (0, 8), (2, 6), (6, 6), (8, 4), (8, 2), (6, 0), (2, 0), (0, 2)]],
        'T' => &[&[(0, 12), (8, 12)], &[(4, 12), (4, 0)]],
        'U' => &[&[(0, 12), (0, 2), (2, 0), (6, 0), (8, 2), (8, 12)]],
        'V' => &[&[(0, 12), (4, 0), (8, 12)]],
        'W' => &[&[(0, 12), (2, 0), (4, 6), (6, 0), (8, 12)]],
        'X' => &[&[(0, 0), (8, 12)], &[(0, 12), (8, 0)]],
        'Y' => &[&[(0, 12), (4, 6), (8, 12)], &[(4, 6), (4, 0)]],
        'Z' => &[&[(0, 12), (8, 12), (0, 0), (8, 0)]],
        '0' => &[&[(2, 0), (1, 2), (1, 10), (2, 12), (6, 12), (7, 10), (7, 2), (6, 0), (2, 0)]],
        '1' => &[&[(2, 10), (4, 12), (4, 0)], &[(2, 0), (6, 0)]],
        '2' => &[&[(0, 10), (2, 12), (6, 12), (8, 10), (8, 8), (0, 0), (8, 0)]],
        '3' => &[&[(0, 10), (2, 12), (6, 12), (8, 10), (8, 8), (6, 6), (8, 4), (8, 2), (6, 0), (2, 0), (0, 2)], &[(3, 6), (6, 6)]],
        '4' => &[&[(6, 0), (6, 12), (0, 4), (8, 4)]],
        '5' => &[&[(8, 12), (0, 12), (0, 7), (6, 7), (8, 5), (8, 2), (6, 0), (2, 0), (0, 2)]],
        '6' => &[&[(8, 10), (6, 12), (2, 12), (0, 10), (0, 2), (2, 0), (6, 0), (8, 2), (8, 5), (6, 7), (2, 7), (0, 5)]],
        '7' => &[&[(0, 12), (8, 12), (3, 0)]],
        '8' => &[&[
            (2, 6),
            (0, 8),
            (0, 10),
            (2, 12),
            (6, 12),
            (8, 10),
            (8, 8),
            (6, 6),
            (2, 6),
            (0, 4),
            (0, 2),
            (2, 0),
            (6, 0),
            (8, 2),
            (8, 4),
            (6, 6),
        ]],
        '9' => &[&[(0, 2), (2, 0), (6, 0), (8, 2), (8, 10), (6, 12), (2, 12), (0, 10), (0, 7), (2, 5), (6, 5), (8, 7)]],
        ' ' => &[],
        '.' => &[&[(3, 0), (4, 0), (4, 1), (3, 1), (3, 0)]],
        ',' => &[&[(4, 1), (4, 0), (3, -2)]],
        '-' => &[&[(1, 6), (7, 6)]],
        '+' => &[&[(4, 2), (4, 10)], &[(0, 6), (8, 6)]],
        '=' => &[&[(1, 4), (7, 4)], &[(1, 8), (7, 8)]],
        '_' => &[&[(0, -1), (8, -1)]],
        '/' => &[&[(0, 0), (8, 12)]],
        '(' => &[&[(6, 12), (4, 9), (4, 3), (6, 0)]],
        '{' => &[&[(6, 12), (4, 11), (4, 7), (2, 6), (4, 5), (4, 1), (6, 0)]],
        '}' => &[&[(2, 12), (4, 11), (4, 7), (6, 6), (4, 5), (4, 1), (2, 0)]],
        ')' => &[&[(2, 12), (4, 9), (4, 3), (2, 0)]],
        ':' => &[&[(4, 2), (4, 3)], &[(4, 8), (4, 9)]],
        '\'' => &[&[(4, 12), (4, 9)]],
        '"' => &[&[(3, 12), (3, 9)], &[(5, 12), (5, 9)]],
        '#' => &[&[(2, 0), (3, 12)], &[(5, 0), (6, 12)], &[(0, 4), (8, 4)], &[(0, 8), (8, 8)]],
        '×' => &[&[(1, 2), (7, 10)], &[(1, 10), (7, 2)]],
        '²' => &[&[(2, 11), (3, 12), (5, 12), (6, 11), (6, 10), (2, 7), (6, 7)]],
        '°' => &[&[(3, 12), (5, 12), (6, 11), (6, 9), (5, 8), (3, 8), (2, 9), (2, 11), (3, 12)]],
        '±' => &[&[(4, 5), (4, 11)], &[(1, 8), (7, 8)], &[(1, 2), (7, 2)]],
        // Cyrillic: the capitals the Latin alphabet shares are its strokes.
        'А' => glyph('A'),
        'В' => glyph('B'),
        'Е' => glyph('E'),
        'К' => glyph('K'),
        'М' => glyph('M'),
        'Н' => glyph('H'),
        'О' => glyph('O'),
        'Р' => glyph('P'),
        'С' => glyph('C'),
        'Т' => glyph('T'),
        'Х' => glyph('X'),
        'З' => glyph('3'),
        'Б' => &[&[(8, 12), (0, 12), (0, 0), (6, 0), (8, 2), (8, 5), (6, 7), (0, 7)]],
        'Г' => &[&[(8, 12), (0, 12), (0, 0)]],
        'Д' => &[&[(0, -2), (0, 0), (8, 0), (8, -2)], &[(1, 0), (3, 12), (7, 12), (7, 0)]],
        'Ж' => &[&[(4, 0), (4, 12)], &[(0, 12), (4, 6), (8, 12)], &[(0, 0), (4, 6), (8, 0)]],
        'И' => &[&[(0, 12), (0, 0), (8, 12), (8, 0)]],
        'Л' => &[&[(0, 0), (2, 2), (3, 12), (8, 12), (8, 0)]],
        'П' => &[&[(0, 0), (0, 12), (8, 12), (8, 0)]],
        'У' => &[&[(0, 12), (4, 5)], &[(8, 12), (3, 0), (1, 0)]],
        'Ф' => &[&[(4, 0), (4, 12)], &[(4, 10), (1, 10), (0, 9), (0, 5), (1, 4), (7, 4), (8, 5), (8, 9), (7, 10), (4, 10)]],
        'Ц' => &[&[(0, 12), (0, 0), (7, 0), (7, 12)], &[(7, 0), (8, 0), (8, -2)]],
        'Ч' => &[&[(0, 12), (0, 8), (2, 6), (8, 6)], &[(8, 12), (8, 0)]],
        'Ш' => &[&[(0, 12), (0, 0), (8, 0), (8, 12)], &[(4, 12), (4, 0)]],
        'Щ' => &[&[(0, 12), (0, 0), (6, 0), (6, 12)], &[(3, 12), (3, 0)], &[(6, 0), (8, 0), (8, -2)]],
        'Ъ' => &[&[(0, 12), (2, 12), (2, 0), (6, 0), (8, 2), (8, 5), (6, 7), (2, 7)]],
        'Ы' => &[&[(0, 12), (0, 0), (4, 0), (6, 2), (6, 5), (4, 7), (0, 7)], &[(8, 12), (8, 0)]],
        'Ь' => &[&[(0, 12), (0, 0), (6, 0), (8, 2), (8, 5), (6, 7), (0, 7)]],
        'Э' => &[&[(0, 10), (2, 12), (6, 12), (8, 10), (8, 2), (6, 0), (2, 0), (0, 2)], &[(3, 6), (8, 6)]],
        'Ю' => &[&[(0, 12), (0, 0)], &[(0, 6), (3, 6)], &[(5, 12), (3, 10), (3, 2), (5, 0), (6, 0), (8, 2), (8, 10), (6, 12), (5, 12)]],
        'Я' => &[&[(8, 0), (8, 12), (2, 12), (0, 10), (0, 8), (2, 6), (8, 6)], &[(4, 6), (0, 0)]],
        _ => QUESTION,
    }
}

/// An accented capital as its base letter and accent.
fn accent(c: char) -> Option<(char, Glyph)> {
    const ACUTE: Glyph = &[&[(3, 13), (5, 15)]];
    const GRAVE: Glyph = &[&[(3, 15), (5, 13)]];
    const CIRCUMFLEX: Glyph = &[&[(2, 13), (4, 15), (6, 13)]];
    const TILDE: Glyph = &[&[(1, 13), (3, 15), (5, 13), (7, 15)]];
    const DIAERESIS: Glyph = &[&[(2, 14), (2, 15)], &[(6, 14), (6, 15)]];
    const CEDILLA: Glyph = &[&[(4, 0), (5, -1), (3, -3)]];
    const BREVE: Glyph = &[&[(2, 15), (3, 13), (5, 13), (6, 15)]];
    Some(match c {
        'Ё' => ('Е', DIAERESIS),
        'Й' => ('И', BREVE),
        'Á' | 'É' | 'Í' | 'Ó' | 'Ú' | 'Ý' => (base(c)?, ACUTE),
        'À' | 'È' | 'Ì' | 'Ò' | 'Ù' => (base(c)?, GRAVE),
        'Â' | 'Ê' | 'Î' | 'Ô' | 'Û' => (base(c)?, CIRCUMFLEX),
        'Ã' | 'Õ' | 'Ñ' => (base(c)?, TILDE),
        'Ä' | 'Ë' | 'Ï' | 'Ö' | 'Ü' => (base(c)?, DIAERESIS),
        'Ç' => ('C', CEDILLA),
        _ => return None,
    })
}

fn base(c: char) -> Option<char> {
    Some(match c {
        'Á' | 'À' | 'Â' | 'Ã' | 'Ä' => 'A',
        'É' | 'È' | 'Ê' | 'Ë' => 'E',
        'Í' | 'Ì' | 'Î' | 'Ï' => 'I',
        'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ö' => 'O',
        'Ú' | 'Ù' | 'Û' | 'Ü' => 'U',
        'Ý' => 'Y',
        'Ñ' => 'N',
        _ => return None,
    })
}

/// Grid units from one character to the next.
const ADVANCE: f32 = 11.0;

/// How wide `text` is at `height` px, in the lettering font if there's one.
pub fn width(text: &str, height: f32) -> f32 {
    if let Some(face) = face() {
        return face_width(face, text, height);
    }
    let n = text.chars().count() as f32;
    if n == 0.0 { 0.0 } else { (n * ADVANCE - 3.0) * height / 12.0 }
}

/// Polylines drawing `text` with capitals `height` px tall, from `(x, y)` on the baseline, y down.
pub fn strokes(text: &str, x: f32, y: f32, height: f32) -> Vec<Vec<(f32, f32)>> {
    let k = height / 12.0;
    let mut out = Vec::new();
    for (i, c) in text.chars().flat_map(char::to_uppercase).enumerate() {
        let x0 = x + i as f32 * ADVANCE * k;
        let (letter, mark): (Glyph, Glyph) = match accent(c) {
            Some((b, m)) => (glyph(b), m),
            None => (glyph(c), &[]),
        };
        for line in letter.iter().chain(mark) {
            out.push(line.iter().map(|&(gx, gy)| (x0 + f32::from(gx) * k, y - f32::from(gy) * k)).collect());
        }
    }
    out
}

/// The TrueType font the lettering is set in instead of the strokes above, read once from the file
/// `BIMCRAFT_LETTERING` names (ISOCPEUR, say): a drawing font isn't ours to bundle, so it's only
/// used where it's installed. Unset, or a file that isn't a font, keeps the strokes.
fn face() -> Option<&'static Face<'static>> {
    static FACE: OnceLock<Option<Face<'static>>> = OnceLock::new();
    FACE.get_or_init(|| {
        let path = std::env::var_os("BIMCRAFT_LETTERING").filter(|p| !p.is_empty())?;
        let data: &'static [u8] = std::fs::read(path).ok()?.leak();
        Face::parse(data, 0).ok()
    })
    .as_ref()
}

/// What `text` is drawn with, from `(x, y)` on the baseline, y down, capitals `height` px tall.
pub enum Letters {
    /// The strokes above, to draw as lines.
    Lines(Vec<Vec<(f32, f32)>>),
    /// The outlines of a TrueType font, rings to fill (non-zero) as one shape.
    Rings(Vec<Vec<(f32, f32)>>),
}

impl Letters {
    /// The letters as lines to draw, a font's rings closed: where nothing can be filled.
    pub fn lines(self) -> Vec<Vec<(f32, f32)>> {
        match self {
            Letters::Lines(lines) => lines,
            Letters::Rings(rings) => rings
                .into_iter()
                .map(|mut r| {
                    r.extend(r.first().copied());
                    r
                })
                .collect(),
        }
    }
}

/// `text` in the lettering font if there's one, else in strokes; see [`Letters`].
pub fn letters(text: &str, x: f32, y: f32, height: f32) -> Letters {
    match face() {
        Some(face) => Letters::Rings(outlines(face, text, x, y, height)),
        None => Letters::Lines(strokes(text, x, y, height)),
    }
}

/// Font units a capital stands: the font's cap height, or seven tenths of its em.
fn cap(face: &Face) -> f32 {
    face.capital_height().filter(|h| *h > 0).map_or(f32::from(face.units_per_em()) * 0.7, f32::from)
}

/// The font's glyph for `c`, its `?` for a character it lacks.
fn index(face: &Face, c: char) -> Option<GlyphId> {
    face.glyph_index(c).or_else(|| face.glyph_index('?'))
}

/// How far, in font units, the pen moves past `c`.
fn advance(face: &Face, c: char) -> f32 {
    index(face, c).and_then(|g| face.glyph_hor_advance(g)).map_or(0.0, f32::from)
}

/// How wide `text` is in `face` at `height` px.
fn face_width(face: &Face, text: &str, height: f32) -> f32 {
    text.chars().map(|c| advance(face, c)).sum::<f32>() * height / cap(face)
}

/// The rings of `face`'s outlines of `text`, as [`letters`] places them.
fn outlines(face: &Face, text: &str, x: f32, y: f32, height: f32) -> Vec<Vec<(f32, f32)>> {
    let mut pen = Pen { rings: Vec::new(), at: (x, y), k: height / cap(face) };
    for c in text.chars() {
        if let Some(g) = index(face, c) {
            face.outline_glyph(g, &mut pen);
        }
        pen.at.0 += advance(face, c) * pen.k;
    }
    pen.rings.retain(|r| r.len() > 2);
    pen.rings
}

/// Pieces each curve of an outline is drawn in.
const STEPS: u16 = 6;

/// Collects a glyph's outline as rings in px, placed `at` its origin and scaled `k` px a font unit.
struct Pen {
    rings: Vec<Vec<(f32, f32)>>,
    at: (f32, f32),
    k: f32,
}

impl Pen {
    fn px(&self, x: f32, y: f32) -> (f32, f32) {
        (self.at.0 + x * self.k, self.at.1 - y * self.k)
    }

    /// The Bézier curve from the ring's last point through `rest`'s control points to its end, flattened.
    fn bend(&mut self, rest: &[(f32, f32)]) {
        let Some(ring) = self.rings.last_mut() else { return };
        let Some(&from) = ring.last() else { return };
        for i in 1..=STEPS {
            let t = f32::from(i) / f32::from(STEPS);
            let mut pts: Vec<(f32, f32)> = std::iter::once(from).chain(rest.iter().copied()).collect();
            while pts.len() > 1 {
                pts = pts
                    .windows(2)
                    .filter_map(|w| match w {
                        [a, b] => Some((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)),
                        _ => None,
                    })
                    .collect();
            }
            ring.extend(pts.first().copied());
        }
    }
}

impl OutlineBuilder for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = self.px(x, y);
        self.rings.push(vec![p]);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.px(x, y);
        if let Some(ring) = self.rings.last_mut() {
            ring.push(p);
        }
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let rest = [self.px(x1, y1), self.px(x, y)];
        self.bend(&rest);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let rest = [self.px(x1, y1), self.px(x2, y2), self.px(x, y)];
        self.bend(&rest);
    }

    fn close(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A font to set lettering in: the interface's own, as any TrueType font would do.
    fn plex() -> Face<'static> {
        let data = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/ibm-plex-mono/IBMPlexMono-Regular.ttf"));
        Face::parse(data, 0).unwrap_or_else(|e| panic!("the bundled font reads: {e}"))
    }

    fn span(rings: &[Vec<(f32, f32)>], axis: fn(&(f32, f32)) -> f32) -> (f32, f32) {
        rings.iter().flatten().map(axis).fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)))
    }

    #[test]
    fn a_fonts_capitals_stand_on_the_baseline_as_tall_as_asked_and_lower_case_is_its_own() {
        let face = plex();
        let h = outlines(&face, "H", 100.0, 50.0, 20.0);
        let (top, foot) = span(&h, |p| p.1);
        assert!((foot - 50.0).abs() < 0.01 && (top - 30.0).abs() < 0.01, "H from {top} to {foot}");
        assert!(span(&h, |p| p.0).0 >= 100.0);
        // An O has its hole: two rings, filled as one shape.
        assert_eq!(outlines(&face, "O", 0.0, 0.0, 20.0).len(), 2);
        assert_ne!(outlines(&face, "o", 0.0, 0.0, 20.0), outlines(&face, "O", 0.0, 0.0, 20.0));
        assert!(!outlines(&face, "Щ", 0.0, 0.0, 20.0).is_empty() || face.glyph_index('Щ').is_none());
        assert!(outlines(&face, " ", 0.0, 0.0, 20.0).is_empty());
    }

    #[test]
    fn a_fonts_letters_follow_one_another_by_their_advances_and_that_is_the_width() {
        let face = plex();
        let one = face_width(&face, "H", 20.0);
        assert!(one > 0.0);
        assert!((face_width(&face, "HHH", 20.0) - 3.0 * one).abs() < 1e-3);
        let (a, b) = (outlines(&face, "H", 0.0, 0.0, 20.0), outlines(&face, "HH", 0.0, 0.0, 20.0));
        assert_eq!(b.len(), 2 * a.len());
        let moved: Vec<Vec<(f32, f32)>> = a.iter().map(|r| r.iter().map(|&(x, y)| (x + one, y)).collect()).collect();
        for (r, m) in b[a.len()..].iter().zip(&moved) {
            assert!(r.iter().zip(m).all(|(p, q)| (p.0 - q.0).abs() < 1e-3 && (p.1 - q.1).abs() < 1e-3));
        }
    }

    #[test]
    fn rings_drawn_as_lines_are_closed_and_strokes_stay_as_they_are() {
        let ring = vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)];
        assert_eq!(Letters::Rings(vec![ring.clone()]).lines(), vec![vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 0.0)]]);
        assert_eq!(Letters::Lines(vec![ring.clone()]).lines(), vec![ring]);
    }

    #[test]
    fn lower_case_is_drawn_as_capitals_and_unknown_characters_as_a_question_mark() {
        assert_eq!(strokes("living", 0.0, 0.0, 12.0), strokes("LIVING", 0.0, 0.0, 12.0));
        assert_eq!(strokes("€", 0.0, 0.0, 12.0), strokes("?", 0.0, 0.0, 12.0));
        assert!(strokes(" ", 0.0, 0.0, 12.0).is_empty());
    }

    #[test]
    fn accented_letters_are_their_base_letter_plus_an_accent() {
        let plain = strokes("A", 0.0, 0.0, 12.0);
        let acute = strokes("á", 0.0, 0.0, 12.0);
        assert_eq!(acute.len(), plain.len() + 1);
        assert_eq!(acute[..plain.len()], plain[..]);
    }

    #[test]
    fn cyrillic_capitals_are_drawn_each_its_own_and_lower_case_as_them() {
        let unknown = strokes("?", 0.0, 0.0, 12.0);
        let all = "АБВГДЕЁЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯ";
        let drawn: Vec<_> = all.chars().map(|c| strokes(&c.to_string(), 0.0, 0.0, 12.0)).collect();
        for (c, s) in all.chars().zip(&drawn) {
            assert!(!s.is_empty() && *s != unknown, "{c} is not drawn");
        }
        for (i, a) in drawn.iter().enumerate() {
            assert!(drawn.iter().skip(i + 1).all(|b| b != a), "{} looks like another", all.chars().nth(i).unwrap_or(' '));
        }
        // The letters the Latin alphabet shares are the same strokes.
        assert_eq!(strokes("АВЕКМНОРСТХ", 0.0, 0.0, 12.0), strokes("ABEKMHOPCTX", 0.0, 0.0, 12.0));
        assert_eq!(strokes("шлейф ёж", 0.0, 0.0, 12.0), strokes("ШЛЕЙФ ЁЖ", 0.0, 0.0, 12.0));
        // Й and Ё are И and Е with their marks over them.
        let (i, short) = (strokes("И", 0.0, 0.0, 12.0), strokes("Й", 0.0, 0.0, 12.0));
        assert_eq!(short[..i.len()], i[..]);
        let (e, yo) = (strokes("Е", 0.0, 0.0, 12.0), strokes("Ё", 0.0, 0.0, 12.0));
        assert_eq!(yo[..e.len()], e[..]);
    }

    #[test]
    fn text_is_scaled_to_its_height_from_the_baseline() {
        let s = strokes("I", 10.0, 100.0, 24.0);
        let ys: Vec<f32> = s.iter().flatten().map(|p| p.1).collect();
        assert_eq!(ys.iter().copied().fold(f32::MAX, f32::min), 76.0);
        assert_eq!(ys.iter().copied().fold(f32::MIN, f32::max), 100.0);
        assert_eq!(width("II", 12.0), 19.0);
    }
}
