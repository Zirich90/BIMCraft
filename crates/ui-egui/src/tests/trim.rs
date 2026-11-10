use super::*;
use bimcraft_core::{curves::Curve, sketch::SketchLine};

fn sketching(lines: Vec<SketchLine>) -> Harness<'static, BimApp> {
    let mut h = app(BOX);
    h.state_mut().start_sketch(crate::sketch::Of::Floor(100000), lines);
    h.run();
    h
}

fn line(a: [f64; 2], b: [f64; 2]) -> SketchLine {
    SketchLine::new(P2(a[0], a[1]), P2(b[0], b[1]))
}

fn curves(h: &Harness<BimApp>) -> Vec<Curve> {
    h.state().sketch.as_ref().unwrap().lines.iter().map(|l| l.curve.clone()).collect()
}

#[test]
fn sketch_trim_multiple_keeps_the_boundary_for_each_target_and_undoes_each_edit() {
    let mut h =
        sketching(vec![line([1000., 1000.], [6000., 1000.]), line([3000., 2000.], [3000., 4000.]), line([5000., 2000.], [5000., 4000.])]);
    let before = curves(&h);
    let model = Arc::clone(&h.state().session.model);
    topmost(&mut h, "Modify");
    press(&mut h, "Trim/Extend Multiple Elements");
    click(&mut h, P2(2000., 1000.));
    click(&mut h, P2(3000., 3000.));
    assert_eq!(curves(&h)[1], Curve::line(P2(3000., 1000.), P2(3000., 4000.)));
    assert!(h.state().drawing.trim.is_some(), "the boundary stays picked");
    click(&mut h, P2(5000., 3000.));
    assert_eq!(curves(&h)[2], Curve::line(P2(5000., 1000.), P2(5000., 4000.)));
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(curves(&h)[2], before[2]);
    assert_ne!(curves(&h)[1], before[1]);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(curves(&h), before);
    assert_eq!(h.state().session.model.elements, model.elements);
}

#[test]
fn sketch_trim_corner_keeps_the_first_pick_after_a_parallel_line_and_clears_a_successful_error() {
    let mut h =
        sketching(vec![line([1000., 1000.], [2500., 1000.]), line([1000., 2000.], [2500., 2000.]), line([3000., 1500.], [3000., 4000.])]);
    shortcut(&mut h, "TR");
    click(&mut h, P2(1500., 1000.));
    click(&mut h, P2(1500., 2000.));
    assert!(h.state().status.error);
    assert!(h.state().drawing.trim.is_some(), "a refused second line must not lose the first");
    click(&mut h, P2(3000., 3000.));
    assert_eq!(curves(&h)[0], Curve::line(P2(1000., 1000.), P2(3000., 1000.)));
    assert_eq!(curves(&h)[2], Curve::line(P2(3000., 1000.), P2(3000., 4000.)));
    assert!(!h.state().status.error, "the successful trim clears the previous error");
    assert!(h.state().drawing.trim.is_none());
}

#[test]
fn sketch_trim_single_keeps_its_boundary_when_the_target_is_refused() {
    let mut h =
        sketching(vec![line([1000., 1000.], [6000., 1000.]), line([1000., 2000.], [2500., 2000.]), line([3000., 2000.], [3000., 4000.])]);
    topmost(&mut h, "Modify");
    press(&mut h, "Trim/Extend Single Element");
    click(&mut h, P2(2000., 1000.));
    click(&mut h, P2(2000., 2000.));
    assert!(h.state().status.error);
    assert!(h.state().drawing.trim.is_some());
    click(&mut h, P2(3000., 3000.));
    assert_eq!(curves(&h)[2], Curve::line(P2(3000., 1000.), P2(3000., 4000.)));
    assert!(h.state().drawing.trim.is_none());
    assert!(!h.state().status.error);
}

#[test]
fn sketch_trim_corner_cuts_crossing_lines_to_close_a_floor_boundary_and_is_one_sketch_undo_step() {
    let mut h = sketching(vec![
        line([1000., 1000.], [6000., 1000.]),
        line([5000., 0.], [5000., 4000.]),
        line([5000., 4000.], [1000., 4000.]),
        line([1000., 4000.], [1000., 1000.]),
    ]);
    let before = curves(&h);
    let model = Arc::clone(&h.state().session.model);
    press(&mut h, "Trim/Extend to Corner");
    assert!(h.state().prompt().contains("first sketch line"));
    click(&mut h, P2(3000., 1000.));
    assert!(h.state().prompt().contains("second sketch line"));
    click(&mut h, P2(5000., 3000.));
    assert_eq!(curves(&h)[0], Curve::line(P2(1000., 1000.), P2(5000., 1000.)));
    assert_eq!(curves(&h)[1], Curve::line(P2(5000., 1000.), P2(5000., 4000.)));
    assert_eq!(h.state().session.model.elements, model.elements);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(curves(&h), before);
    click(&mut h, P2(3000., 1000.));
    click(&mut h, P2(5000., 3000.));
    press(&mut h, "Finish Edit Mode");
    assert!(h.state().sketch.is_none(), "{:?}", h.state().status);
    assert_eq!(field(&h, ids(&h, "floor")[0], "area_m2"), json!(12.));
}

#[test]
fn sketch_trim_highlights_candidates_and_escape_clears_the_first_pick() {
    let mut h = sketching(vec![line([1000., 1000.], [6000., 1000.]), line([3000., 2000.], [3000., 4000.])]);
    for button in ["Trim/Extend to Corner", "Trim/Extend Single Element", "Trim/Extend Multiple Elements"] {
        press(&mut h, button);
        let a = h.state().screen(P2(1000., 1000.));
        let b = h.state().screen(P2(6000., 1000.));
        let at = h.state().screen(P2(2000., 1000.));
        h.hover_at(at);
        h.run();
        assert!(h.output().shapes.iter().any(|s| matches!(&s.shape,
            egui::Shape::Path(p) if p.stroke.width>=3.0 && p.points.len()==2 && p.points[0].distance(a)<0.2 && p.points[1].distance(b)<0.2
        )),"{button}: the sketch candidate is highlighted");
        click(&mut h, P2(2000., 1000.));
        assert!(h.state().drawing.trim.is_some());
        key(&mut h, Key::Escape);
        assert!(matches!(h.state().tool, Tool::Trim(_) | Tool::TrimTo { .. }));
        assert!(h.state().drawing.trim.is_none());
        assert!(h.state().sketch.is_some());
    }
}

#[test]
fn sketch_trim_multiple_drops_a_boundary_that_other_edits_remove_or_renumber() {
    let mut h = sketching(vec![line([1000., 1000.], [6000., 1000.]), line([3000., 2000.], [3000., 4000.])]);
    press(&mut h, "Trim/Extend Multiple Elements");
    click(&mut h, P2(2000., 1000.));
    assert!(h.state().drawing.trim.is_some());
    h.state_mut().sketch.as_mut().unwrap().picked.insert(0);
    key(&mut h, Key::Delete);
    assert!(h.state().drawing.trim.is_none(), "a saved index cannot point at a different line after deletion");
    assert_eq!(h.state().sketch.as_ref().unwrap().lines.len(), 1);
}
