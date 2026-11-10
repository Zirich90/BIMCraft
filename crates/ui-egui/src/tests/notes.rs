//! Text written where it goes: the Text tool's click opens a note on the canvas, typing writes it, Shift+Enter breaks a
//! line, Enter finishes; a double click on a note rewrites it; Override Formatting lets Ctrl+B, Ctrl+I and Ctrl+U make a
//! part bold, italic or underlined, which a new type keeps; Edit Type opens Text Type Properties, where a type is renamed.

use super::*;

fn text_tool(h: &mut Harness<BimApp>) {
    press(h, "Annotate");
    press(h, "Text");
}

fn typed(h: &mut Harness<BimApp>, s: &str) {
    h.event(Event::Text(s.into()));
    h.run();
}

fn chord(h: &mut Harness<BimApp>, modifiers: Modifiers, k: Key) {
    h.key_press_modifiers(modifiers, k);
    h.run();
}

fn note(h: &Harness<BimApp>, id: Id) -> bimcraft_core::model::Note {
    match h.state().session.model.elements.get(id) {
        Some(Element::Text(n)) => n.clone(),
        e => panic!("{id} is {e:?}"),
    }
}

#[test]
fn a_click_of_the_text_tool_opens_a_note_on_the_canvas_where_typing_writes_it_and_enter_finishes_it() {
    let mut h = app(BOX);
    text_tool(&mut h);
    click(&mut h, P2(2000.0, 2000.0));
    assert!(h.state().note_edit.is_some(), "{}", h.state().status.text);
    // The tool's shortcuts are letters it takes as text now: WA is no wall.
    typed(&mut h, "WA ");
    typed(&mut h, "Árvíztűrő");
    chord(&mut h, Modifiers::SHIFT, Key::Enter);
    typed(&mut h, "Жx");
    key(&mut h, Key::Backspace);
    assert_eq!(h.state().tool, Tool::Text);
    assert!(ids(&h, "text").is_empty(), "nothing is made until it's finished");
    key(&mut h, Key::Enter);
    let notes = ids(&h, "text");
    assert_eq!(notes.len(), 1, "{}", h.state().status.text);
    let n = note(&h, notes[0]);
    assert_eq!((n.text.as_str(), n.at, n.type_name.as_str()), ("WA Árvíztűrő\nЖ", P2(2000.0, 2000.0), "2.5mm"));
    assert!(h.state().note_edit.is_none() && h.state().tool == Tool::Text && ids(&h, "wall").len() == 4);
    // One step undoes it all.
    chord(&mut h, Modifiers::COMMAND, Key::Z);
    assert!(ids(&h, "text").is_empty());
}

#[test]
fn a_click_outside_the_note_finishes_it_and_an_empty_one_makes_nothing() {
    let mut h = app(BOX);
    text_tool(&mut h);
    click(&mut h, P2(1000.0, 1000.0));
    click(&mut h, P2(6000.0, 5000.0));
    assert!(ids(&h, "text").is_empty(), "an empty note is no note");
    // That click opened another; written in, a click away makes it.
    typed(&mut h, "Hall");
    click(&mut h, P2(1000.0, 1000.0));
    let notes = ids(&h, "text");
    assert_eq!(notes.len(), 1);
    assert_eq!(note(&h, notes[0]).at, P2(6000.0, 5000.0));
}

#[test]
fn a_double_click_on_a_note_rewrites_it_and_overridden_parts_keep_their_formatting_through_a_new_type() {
    let script = format!("{BOX}\ntext {{\"text\": \"Bold word\", \"at\": [2000, 3000], \"type\": \"5mm\"}}");
    let mut h = app_at(&script, 0.05);
    let id = ids(&h, "text")[0];
    double_click(&mut h, P2(2400.0, 2800.0));
    let e = h.state().note_edit.clone().unwrap_or_else(|| panic!("no editor: {}", h.state().status.text));
    assert_eq!((e.id, e.selected().as_str()), (Some(id), "Bold word"), "it opens with all of it selected");
    // Formatting asks for Override Formatting first.
    chord(&mut h, Modifiers::COMMAND, Key::B);
    assert!(h.state().status.error);
    // The Options Bar's, at the top of the view (Properties has one too).
    let at =
        h.query_all_by_role_and_label(Role::CheckBox, "Override Formatting").map(|n| n.rect()).min_by(|a, b| a.top().total_cmp(&b.top()));
    click_at(&mut h, at.unwrap().center());
    key(&mut h, Key::Home);
    for _ in 0..4 {
        chord(&mut h, Modifiers::SHIFT, Key::ArrowRight);
    }
    chord(&mut h, Modifiers::COMMAND, Key::B);
    key(&mut h, Key::End);
    typed(&mut h, "s");
    key(&mut h, Key::Enter);
    let n = note(&h, id);
    assert_eq!((n.text.as_str(), n.override_format), ("Bold words", true));
    assert_eq!((n.runs.len(), n.runs[0].from, n.runs[0].to, n.runs[0].bold), (1, 0, 4, Some(true)));
    // Another type in the Type Selector keeps them.
    press(&mut h, "Type Selector");
    press(&mut h, "3.5mm");
    let n = note(&h, id);
    assert_eq!((n.type_name.as_str(), n.runs[0].bold), ("3.5mm", Some(true)));
}

#[test]
fn edit_type_on_a_note_opens_text_type_properties_where_its_font_and_border_change_every_note_of_it() {
    let script = format!("{BOX}\ntext {{\"text\": \"Note\", \"at\": [2000, 3000]}}");
    let mut h = app(&script);
    let id = ids(&h, "text")[0];
    click(&mut h, P2(2100.0, 2950.0));
    assert_eq!(h.state().selection, vec![id]);
    press(&mut h, "Edit Type");
    assert!(h.state().text_type_dialog.is_some());
    // Where it is from its first frame, so a click lands on the row it was meant for.
    let at = |h: &Harness<BimApp>| h.ctx.memory(|m| m.area_rect(egui::Id::new("text-type-properties")));
    let first = at(&h);
    h.step();
    assert_eq!(at(&h), first, "the dialog doesn't move after it opens");
    pick(&mut h, Role::CheckBox, "Show Border");
    // Rename, as Revit's: the note follows.
    press(&mut h, "Rename…");
    type_into(&mut h, "New name", "Small");
    press(&mut h, "Rename");
    assert!(h.state().text_type_dialog.as_ref().is_some_and(|d| d.error().is_none()));
    press(&mut h, "OK");
    assert!(h.state().text_type_dialog.is_none(), "{}", h.state().status.text);
    assert!(h.state().session.model.text_types["Small"].border, "what was typed before the rename is kept, and applied");
    assert!(matches!(h.state().session.model.elements.get(id), Some(Element::Text(n)) if n.type_name == "Small"));
}
