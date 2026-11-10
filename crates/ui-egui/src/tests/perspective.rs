//! The 3D view in perspective: Properties > Camera > Projection turns it from Axonometric to Perspective and back, types the
//! lens's focal length and the eye's height, and the view is picked and edited as ever.

use super::*;

fn lens(h: &Harness<BimApp>) -> Option<f64> {
    let m = &h.state().session.model;
    match m.default_3d().and_then(|v| m.elements.get(v)) {
        Some(Element::View3D(v)) => v.lens(),
        _ => None,
    }
}

fn camera(h: &Harness<BimApp>) -> bimcraft_core::view3d::Camera {
    h.state().orbit.camera.expect("the 3D view has shown")
}

#[test]
fn properties_turn_the_3d_view_to_perspective_type_its_lens_and_eye_and_turn_it_back() {
    let mut h = app(BOX);
    press(&mut h, "Default 3D View");
    assert_eq!((lens(&h), camera(&h).lens), (None, None), "parallel to begin with");
    // Nothing selected, Properties is the view's: its Projection Axonometric; Perspective picked, it is seen through a 35 mm lens.
    assert_eq!(combo_value(&h, "Projection"), "Axonometric");
    choose(&mut h, "Projection", "Perspective");
    assert_eq!(combo_value(&h, "Projection"), "Perspective");
    assert_eq!(lens(&h), Some(bimcraft_core::view3d::LENS));
    let cam = camera(&h);
    assert_eq!(cam.lens, Some(bimcraft_core::view3d::LENS));
    let eye = cam.eye().expect("an eye");
    // A longer lens: the eye stays where it stood.
    type_into(&mut h, "Focal Length", "70");
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(lens(&h), Some(70.0));
    let moved = camera(&h).eye().unwrap();
    assert!((0..3).all(|i| (moved[i] - eye[i]).abs() < 1e-3), "{moved:?} {eye:?}");
    // The eye down to a person's height, looking at the same point.
    type_into(&mut h, "Eye Elevation", "1600");
    h.key_press(Key::Enter);
    h.run();
    let low = camera(&h);
    assert!((low.eye_elevation().unwrap() - 1600.0).abs() < 1e-3, "{:?}", low.eye_elevation());
    assert_eq!(low.target, cam.target);
    // A lens no camera has is refused, and says why.
    type_into(&mut h, "Focal Length", "1");
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(lens(&h), Some(70.0));
    assert!(h.state().status.error);
    // Axonometric again: the lens kept for when it's in perspective again.
    choose(&mut h, "Projection", "Axonometric");
    assert_eq!((lens(&h), camera(&h).lens), (None, None));
    choose(&mut h, "Projection", "Perspective");
    assert_eq!(lens(&h), Some(70.0));
}

#[test]
fn in_perspective_the_3d_view_picks_and_its_section_box_grips_drag_as_ever() {
    let mut h = app(BOX);
    h.state_mut().run("section_box", json!({"min": [-1000, -1000, -1000], "max": [9000, 7000, 4000]}));
    h.state_mut().run("perspective", json!({}));
    press(&mut h, "Default 3D View");
    assert!(camera(&h).lens.is_some());
    // A click on the south wall's face picks it.
    let south = find(&h, |e| matches!(e, Element::Wall(w) if w.start.1 == 0.0 && w.end.1 == 0.0));
    let at = h.state().screen3([5000.0, -100.0, 1000.0]);
    click_at(&mut h, at);
    assert_eq!(h.state().selection, [south]);
    // The top's grip, dragged down to 1.5 m as the picture shows it there.
    let before = h.state().orbit.camera;
    let (a, b) = (h.state().screen3([4000.0, 3000.0, 4000.0]), h.state().screen3([4000.0, 3000.0, 1500.0]));
    drag_screen(&mut h, a, b, PointerButton::Primary);
    let top = h.state().session.model.default_3d().and_then(|v| h.state().session.model.section_box(v)).map(|b| b.1[2]).unwrap();
    assert!((top - 1500.0).abs() <= 20.0, "{top}");
    assert_eq!(h.state().orbit.camera, before, "the grip took the drag, not the camera");
    // Above the box's top there's nothing to pick.
    let at = h.state().screen3([5000.0, -100.0, 2500.0]);
    click_at(&mut h, at);
    assert!(h.state().selection.is_empty(), "{:?}", h.state().selection);
}
