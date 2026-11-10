//! Manage > Technical Standards: configure drawing standards (NBR, ISO, ANSI, EU), adapting
//! line weights, sheet layouts, object styles, and dimensioning conventions.

use super::*;
use bimcraft_core::standards::TechnicalStandard;

#[test]
fn technical_standards_configure_and_adapt_program() {
    let mut h = app(BOX);
    press(&mut h, "Manage");
    press(&mut h, "Technical Standards");
    assert!(h.state().standards_dialog.is_some(), "Technical Standards button opens dialog");

    // Select NBR and press OK
    h.state_mut().standards_dialog.as_mut().unwrap().selected = TechnicalStandard::Nbr;
    press(&mut h, "OK");
    assert!(h.state().standards_dialog.is_none(), "OK closes dialog");

    let m = &h.state().session.model;
    assert_eq!(m.standard(), TechnicalStandard::Nbr);
    assert_eq!(m.object_style("wall").cut, bimcraft_core::pen::Pen(10));
    assert_eq!(m.object_style("column").cut, bimcraft_core::pen::Pen(12));

    // Verify NBR sheet margins
    let margins = m.standard().sheet_margins("A1");
    assert_eq!(margins.left, 25.0, "NBR 16752 25mm left margin");
    assert_eq!(margins.title_width, 178.0, "NBR 16752 max 178mm title block");
    assert!(margins.folding_marks, "NBR 17067 folding marks");

    // Open again, select ANSI, and press Apply
    press(&mut h, "Technical Standards");
    h.state_mut().standards_dialog.as_mut().unwrap().selected = TechnicalStandard::Ansi;
    press_near(&mut h, "Apply", "Cancel");
    assert!(h.state().standards_dialog.is_some(), "Apply keeps dialog open");
    assert_eq!(h.state().session.model.standard(), TechnicalStandard::Ansi);
    assert_eq!(h.state().session.model.units, bimcraft_core::units::Units::FeetInches);

    press(&mut h, "Cancel");
    assert!(h.state().standards_dialog.is_none(), "Cancel closes dialog");
}
