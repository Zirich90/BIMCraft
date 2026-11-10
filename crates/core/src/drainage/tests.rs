//! The drain and vent tables of NBR 8160 as printed, and what they size.

use super::*;

#[test]
fn tabelas_5_6_and_7_are_as_printed() {
    // branches
    assert_eq!([40.0, 50.0, 75.0, 100.0].map(|dn| capacity(dn, 0.01, Role::Branch, 1)), [3.0, 6.0, 20.0, 160.0]);
    // building drains by slope: 0.5, 1, 2, 4 %
    assert_eq!(
        [0.005, 0.01, 0.02, 0.04].map(|s| capacity(100.0, s, Role::Main, 1)),
        [0.0, 180.0, 216.0, 250.0],
        "a blank in Tabela 7 is not allowed, and does not borrow the branch table's 160"
    );
    assert_eq!([0.005, 0.01, 0.02, 0.04].map(|s| capacity(150.0, s, Role::Main, 1)), [0.0, 700.0, 840.0, 1000.0]);
    assert_eq!([0.005, 0.01, 0.02, 0.04].map(|s| capacity(200.0, s, Role::Main, 1)), [1400.0, 1600.0, 1920.0, 2300.0]);
    assert_eq!([0.005, 0.01, 0.02, 0.04].map(|s| capacity(250.0, s, Role::Main, 1)), [2500.0, 2900.0, 3500.0, 4200.0]);
    assert_eq!([0.005, 0.01, 0.02, 0.04].map(|s| capacity(300.0, s, Role::Main, 1)), [3900.0, 4600.0, 5600.0, 6700.0]);
    assert_eq!([0.005, 0.01, 0.02, 0.04].map(|s| capacity(400.0, s, Role::Main, 1)), [7000.0, 8300.0, 10000.0, 12000.0]);
    assert_eq!(smallest(2400.0, 0.005, Role::Main, 1, 0.0), Some(250.0), "DN 200 carries 1400 at 0.5 %");
    // between two slopes: the smaller one, on the safe side
    assert_eq!(capacity(150.0, 0.015, Role::Main, 1), 700.0);
    assert_eq!(capacity(150.0, 0.001, Role::Main, 1), 0.0, "flatter than 0.5 %: none");
    assert_eq!(capacity(100.0, 0.0, Role::Main, 1), 0.0, "no slope: none");
    assert_eq!(capacity(150.0, 0.0, Role::Branch, 1), 0.0, "a branch over DN 100 is not in Tabela 5, it does not borrow Tabela 7");
    assert_eq!(smallest(161.0, 0.01, Role::Branch, 1, 40.0), None);
    assert_eq!(smallest(100.0, 0.005, Role::Main, 1, 100.0), Some(200.0), "DN 100 and 150 are not allowed at 0.5 %");
    assert_eq!(capacity(100.0, 0.0, Role::Main, 1), 0.0, "no slope: none");
    assert_eq!(capacity(150.0, 0.0, Role::Branch, 1), 0.0, "a branch over DN 100 is not in Tabela 5, it does not borrow Tabela 7");
    assert_eq!(smallest(161.0, 0.01, Role::Branch, 1, 40.0), None);
    assert_eq!(smallest(100.0, 0.005, Role::Main, 1, 100.0), Some(200.0), "DN 100 and 150 are not allowed at 0.5 %");
    // stacks
    assert_eq!(
        [40.0, 50.0, 75.0, 100.0, 150.0, 200.0, 250.0, 300.0].map(|dn| capacity(dn, 0.0, Role::Stack, 3)),
        [4.0, 10.0, 30.0, 240.0, 960.0, 2200.0, 3800.0, 6000.0]
    );
    assert_eq!(
        [40.0, 50.0, 75.0, 100.0, 150.0, 200.0, 250.0, 300.0].map(|dn| capacity(dn, 0.0, Role::Stack, 4)),
        [8.0, 24.0, 70.0, 500.0, 1900.0, 3600.0, 5600.0, 8400.0]
    );
    assert_eq!(capacity(125.0, 0.0, Role::Stack, 1), 0.0, "a DN the table does not list");
}

#[test]
fn the_fixtures_are_as_tabela_3() {
    assert_eq!(fixture("wc"), Some((6.0, 100.0)));
    assert_eq!(fixture("lav"), Some((1.0, 40.0)));
    assert_eq!(fixture("sink"), Some((3.0, 50.0)));
    assert_eq!(fixture("shower"), Some((2.0, 40.0)));
    assert_eq!(fixture("laundry"), Some((3.0, 40.0)));
    assert_eq!(fixture("bidet"), None);
}

#[test]
fn the_smallest_drain_for_the_units() {
    assert_eq!(smallest(3.0, 0.01, Role::Branch, 1, 0.0), Some(40.0));
    assert_eq!(smallest(4.0, 0.01, Role::Branch, 1, 0.0), Some(50.0));
    assert_eq!(smallest(21.0, 0.01, Role::Branch, 1, 0.0), Some(100.0));
    assert_eq!(smallest(1.0, 0.01, Role::Branch, 1, 100.0), Some(100.0), "never under the fixture's own drain: a water closet 100");
    assert_eq!(smallest(10.0, 0.01, Role::Main, 1, 0.0), Some(100.0), "a building drain is DN 100 at least");
    assert_eq!(smallest(200.0, 0.01, Role::Main, 1, 0.0), Some(150.0));
    assert_eq!(smallest(200.0, 0.005, Role::Main, 1, 0.0), Some(200.0), "150 has none at 0.5 %");
    assert_eq!(smallest(250.0, 0.04, Role::Main, 1, 0.0), Some(100.0));
    assert_eq!(smallest(251.0, 0.04, Role::Main, 1, 0.0), Some(150.0));
    assert_eq!(smallest(100.0, 0.0, Role::Stack, 2, 0.0), Some(100.0), "a stack of up to three floors: 100 carries 240");
    assert_eq!(smallest(241.0, 0.0, Role::Stack, 3, 0.0), Some(150.0));
    assert_eq!(smallest(241.0, 0.0, Role::Stack, 4, 0.0), Some(100.0), "more than three floors: 100 carries 500");
    assert_eq!(smallest(1.0e6, 0.01, Role::Branch, 1, 0.0), None);
}

#[test]
fn tabela_8_branch_vents() {
    assert_eq!(
        [1.0, 12.0, 13.0, 18.0, 19.0, 36.0].map(|u| branch_vent(u, false)),
        [Some(40.0), Some(40.0), Some(50.0), Some(50.0), Some(75.0), Some(75.0)]
    );
    assert_eq!(branch_vent(37.0, false), None);
    assert_eq!([6.0, 17.0, 18.0, 60.0].map(|u| branch_vent(u, true)), [Some(50.0), Some(50.0), Some(75.0), Some(75.0)]);
    assert_eq!(branch_vent(61.0, true), None);
}

#[test]
fn tabela_2_vent_stacks() {
    // a DN 100 stack of 140 units: vents of 50 up to 8 m, 75 up to 61 m, 100 up to 229 m
    assert_eq!(vent_stack(100.0, 140.0, 8.0), Some(50.0));
    assert_eq!(vent_stack(100.0, 140.0, 8.1), Some(75.0));
    assert_eq!(vent_stack(100.0, 140.0, 61.0), Some(75.0));
    assert_eq!(vent_stack(100.0, 140.0, 62.0), Some(100.0));
    assert_eq!(vent_stack(100.0, 140.0, 230.0), None, "longer than the table");
    assert_eq!(vent_stack(100.0, 141.0, 8.0), Some(75.0), "the next row of units up: 320");
    assert_eq!(vent_stack(100.0, 531.0, 5.0), None, "more units than the table");
    assert_eq!(vent_stack(40.0, 8.0, 46.0), Some(40.0));
    assert_eq!(vent_stack(40.0, 8.0, 47.0), None);
    assert_eq!(vent_stack(300.0, 26000.0, 152.0), Some(300.0));
    assert_eq!(vent_stack(125.0, 10.0, 1.0), None, "a DN the table does not list");
}

#[test]
fn tabela_1_trap_to_vent() {
    assert_eq!([40.0, 50.0, 75.0, 100.0].map(trap_distance), [Some(1000.0), Some(1200.0), Some(1800.0), Some(2400.0)]);
    assert_eq!(trap_distance(150.0), None);
}

mod command {
    use serde_json::{Value, json};

    use crate::engine::Session;

    fn run(s: &mut Session, id: &str, p: Value) -> Value {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
    }

    #[test]
    fn drain_size_gives_the_dn_by_fixtures_or_by_units() {
        let mut s = Session::new();
        let r = run(&mut s, "drain_size", json!({"fixtures": {"wc": 2, "lav": 3}}));
        assert_eq!((&r["units"], &r["dn"], &r["capacity"], &r["minimum_dn"]), (&json!(15.0), &json!(100.0), &json!(160.0), &json!(100.0)));
        let r = run(&mut s, "drain_size", json!({"units": 200, "role": "main", "slope": 0.005}));
        assert_eq!((&r["dn"], &r["capacity"]), (&json!(200.0), &json!(1400.0)));
        let r = run(&mut s, "drain_size", json!({"units": 300, "role": "stack", "floors": 5}));
        assert_eq!((&r["dn"], &r["capacity"]), (&json!(100.0), &json!(500.0)));
        let r = run(&mut s, "drain_size", json!({"units": 3, "min_dn": 75}));
        assert_eq!(r["dn"], json!(75.0), "never smaller than a drain flowing into it");
        assert_eq!(crate::engine::find("drain_size").map(|c| c.undoable), Some(false));
    }

    #[test]
    fn vent_size_gives_a_branch_vent_or_a_vent_stack() {
        let mut s = Session::new();
        assert_eq!(run(&mut s, "vent_size", json!({"units": 15, "wc": true}))["dn"], json!(50.0));
        assert_eq!(run(&mut s, "vent_size", json!({"units": 15}))["dn"], json!(50.0));
        assert_eq!(run(&mut s, "vent_size", json!({"units": 30}))["dn"], json!(75.0));
        let r = run(&mut s, "vent_size", json!({"stack_dn": 100, "units": 140, "length": 20000}));
        assert_eq!(r["dn"], json!(75.0));
        assert_eq!(r["longest_m"], json!(61.0));
        let r = run(&mut s, "vent_size", json!({"units": 4, "trap_dn": 50}));
        assert_eq!(r["trap_to_vent_mm"], json!(1200.0));
    }

    #[test]
    fn a_bad_parameter_says_which() {
        let mut s = Session::new();
        for (id, bad, says) in [
            ("drain_size", json!({}), "units"),
            ("drain_size", json!({"units": 10, "role": "riser"}), "role"),
            ("drain_size", json!({"fixtures": {"bidet": 1}}), "bidet"),
            ("drain_size", json!({"fixtures": {"wc": -1}}), "wc"),
            ("drain_size", json!({"units": 1.0e6}), "tables"),
            ("drain_size", json!({"units": 10, "pipe": 1}), "pipe"),
            ("vent_size", json!({}), "units"),
            ("vent_size", json!({"units": 500}), "table"),
            ("vent_size", json!({"stack_dn": 125, "units": 10, "length": 1000}), "stack_dn"),
            ("vent_size", json!({"stack_dn": 100, "units": 10}), "length"),
            ("vent_size", json!({"units": 4, "trap_dn": 60}), "trap_dn"),
            ("vent_size", json!([]), "object"),
        ] {
            let e = s.execute(id, &bad).expect_err("should fail");
            assert!(e.contains(says), "{id} {bad}: {e}");
        }
    }
}
