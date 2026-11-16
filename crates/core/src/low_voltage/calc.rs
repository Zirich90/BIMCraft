use crate::commands::*;
use crate::engine::{Command, Session};
use crate::model::{Element, Id, MAX_POINTS};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn fire_voltage_drop(_: &mut Session, p: &Value) -> R {
    const C: &str = "fire_voltage_drop";
    const FIELDS: [&str; 4] = ["length_m", "ohm_per_km", "load_ma", "min_v"];
    only(C, p, &["source_v", "segments"])?;
    let source = num(C, p, "source_v")?
        .filter(|value| *value > 0.0 && *value <= crate::electrical::MAX_VOLTS)
        .ok_or_else(|| bad(C, "give a positive source_v within the voltage limit"))?;
    let segments = p["segments"]
        .as_array()
        .filter(|rows| (1..=MAX_POINTS).contains(&rows.len()))
        .ok_or_else(|| bad(C, format!("give 1 to {MAX_POINTS} segments in source order")))?;
    let mut missing = Vec::new();
    let mut electrical = Vec::with_capacity(segments.len());
    let mut minima = Vec::with_capacity(segments.len());
    for (i, row) in segments.iter().enumerate() {
        only(C, row, &FIELDS)?;
        let mut absent = Vec::new();
        let mut values = [0.0; FIELDS.len()];
        for (index, field) in FIELDS.into_iter().enumerate() {
            match num(C, row, field)? {
                Some(value) if value >= 0.0 => values[index] = value,
                Some(_) => return Err(bad(C, format!("segment {}: {field} must be nonnegative", i + 1))),
                None => absent.push(field),
            }
        }
        if !absent.is_empty() {
            missing.push(json!({"segment":i + 1,"fields":absent}));
        }
        let [length, resistance, current, minimum] = values;
        electrical.push((2.0 * length / 1000.0 * resistance, current / 1000.0));
        minima.push(minimum);
    }
    if !missing.is_empty() {
        return Ok(json!({"source_v":source,"segments":null,"missing":missing}));
    }
    let points: Vec<_> = crate::low_voltage::faults::radial_voltage(source, &electrical)
        .into_iter()
        .zip(minima)
        .enumerate()
        .map(|(i, (voltage, minimum))| {
            json!({"segment":i + 1,"voltage_v":voltage,"drop_v":source - voltage,"min_v":minimum,"below_minimum":minimum - voltage > 1e-9})
        })
        .collect();
    Ok(json!({"source_v":source,"segments":points,"missing":missing}))
}

fn fire_line_load(s: &mut Session, p: &Value) -> R {
    const C: &str = "fire_line_load";
    only(C, p, &["id", "loads", "limit_ma", "reserve_percent"])?;
    let m = &s.model;
    let id = element_id(C, m, &p["id"])?;
    let Some(Element::ElectricalLine(line)) = m.elements.get(id) else { return Err(bad(C, "select an electrical line")) };
    let devices: BTreeSet<Id> = line
        .members()
        .chain(line.isolators.iter().copied())
        .filter(|guid| *guid != line.source)
        .filter_map(|guid| m.elements.id(guid))
        .collect();
    let current = |entry: &Value, key: &str| {
        let value = num(C, entry, key)?;
        if value.is_some_and(|ma| ma < 0.0) {
            return Err(bad(C, format!("{key} must be nonnegative")));
        }
        Ok(value)
    };
    let limit = current(p, "limit_ma")?;
    let reserve = num(C, p, "reserve_percent")?.unwrap_or_default();
    if !(0.0..=100.0).contains(&reserve) {
        return Err(bad(C, "reserve_percent must be between 0 and 100"));
    }
    let allowed = limit.map(|ma| ma * (1.0 - reserve / 100.0));
    let mut loads = BTreeMap::new();
    if let Some(entries) = p.get("loads") {
        let entries = entries
            .as_array()
            .filter(|entries| entries.len() <= MAX_POINTS)
            .ok_or_else(|| bad(C, format!("loads must be an array of at most {MAX_POINTS} entries")))?;
        for entry in entries {
            only(C, entry, &["id", "standby_ma", "fire_ma"])?;
            let device = element_id(C, m, &entry["id"])?;
            if !devices.contains(&device) {
                return Err(bad(C, format!("{device} is not a line load device")));
            }
            let values = [current(entry, "standby_ma")?, current(entry, "fire_ma")?];
            if loads.insert(device, values).is_some() {
                return Err(bad(C, format!("duplicate load for device {device}")));
            }
        }
    }
    let mode = |index: usize| {
        let mut known_ma = 0.0;
        let mut missing_ids = Vec::new();
        for device in &devices {
            if let Some(ma) = loads.get(device).and_then(|values| values[index]) {
                known_ma += ma;
            } else {
                missing_ids.push(device);
            }
        }
        let total = missing_ids.is_empty().then_some(known_ma);
        let over_limit = allowed.and_then(|ma| {
            let exceeds = known_ma - ma > 1e-9;
            if exceeds { Some(true) } else { total.map(|_| false) }
        });
        json!({"known_ma":known_ma,"total_ma":total,"missing_ids":missing_ids,"over_limit":over_limit})
    };
    Ok(json!({"id":id,"limit_ma":limit,"reserve_percent":reserve,"allowed_ma":allowed,"standby":mode(0),"fire":mode(1)}))
}

fn fire_battery(_: &mut Session, p: &Value) -> R {
    const C: &str = "fire_battery";
    const INPUTS: [&str; 8] = [
        "standby_a",
        "fire_a",
        "source_a",
        "end_capacity_percent",
        "load_factor",
        "installed_ah",
        "standby_load_factor",
        "fire_load_factor",
    ];
    const STANDBY_H: f64 = 24.0;
    const FIRE_H: f64 = 1.0;
    only(C, p, &INPUTS)?;
    let mut missing = Vec::new();
    let mut values = [None; INPUTS.len()];
    for (i, key) in INPUTS.into_iter().enumerate() {
        let value = num(C, p, key)?;
        if let Some(value) = value {
            let valid = match key {
                "end_capacity_percent" => value > 0.0 && value <= 100.0,
                "load_factor" | "installed_ah" | "standby_load_factor" | "fire_load_factor" => value > 0.0,
                _ => value >= 0.0,
            };
            if !valid {
                return Err(bad(C, format!("invalid {key}: {value}")));
            }
        } else if i < 4 {
            missing.push(key);
        }
        values[i] = value;
    }
    let [standby, fire, source, remaining, load, installed, standby_load, fire_load] = values;
    let separate = standby_load.is_some() || fire_load.is_some();
    let (standby_factor, fire_factor) = if separate {
        if load.is_some() {
            return Err(bad(C, "use load_factor or the two mode-specific load factors"));
        }
        if standby_load.is_none() {
            missing.push("standby_load_factor");
        }
        if fire_load.is_none() {
            missing.push("fire_load_factor");
        }
        (standby_load, fire_load)
    } else {
        if load.is_none() {
            missing.push("load_factor");
        }
        (load, load)
    };
    let aging_factor = remaining.map(|percent| 100.0 / percent);
    let runtime = match (installed, fire, remaining) {
        (Some(capacity), Some(current), Some(percent)) if current > 0.0 => Some(capacity * (percent / 100.0) / current),
        _ => None,
    };
    if aging_factor.is_some_and(|factor| !factor.is_finite()) || runtime.is_some_and(|hours| !hours.is_finite()) {
        return Err(bad(C, "battery inputs must produce finite results"));
    }
    let mut report = json!({
        "standard": if separate { "SP 6.13130.2025 Appendix B, B.2" } else { "SP 6.13130.2025 Appendix B, B.1" },
        "standby_h": STANDBY_H, "fire_h": FIRE_H,
        "installed_ah": installed, "required_ah": null, "fire_runtime_h": runtime,
        "status": "missing_data", "missing": missing,
    });
    if let (Some(standby), Some(fire), Some(source), Some(aging_factor), Some(standby_factor), Some(fire_factor)) =
        (standby, fire, source, aging_factor, standby_factor, fire_factor)
    {
        let standby_ah = (standby + source) * STANDBY_H;
        let fire_ah = (fire + source) * FIRE_H;
        let required_ah = if let Some(load) = load {
            (standby_ah + fire_ah) * load * aging_factor
        } else {
            (standby_ah * standby_factor + fire_ah * fire_factor) * aging_factor
        };
        if !required_ah.is_finite() {
            return Err(bad(C, "end_capacity_percent is too small for a finite calculation"));
        }
        report["standby_ah"] = json!(standby_ah);
        report["fire_ah"] = json!(fire_ah);
        report["aging_factor"] = json!(aging_factor);
        if separate {
            report["standby_load_factor"] = json!(standby_factor);
            report["fire_load_factor"] = json!(fire_factor);
        } else {
            report["load_factor"] = json!(load);
        }
        report["required_ah"] = json!(required_ah);
        report["status"] = json!(match installed {
            Some(capacity) if capacity < required_ah => "insufficient_capacity",
            Some(_) => "sufficient_capacity",
            None => "calculated",
        });
    }
    Ok(report)
}

pub(crate) const FIRE_VOLTAGE_DROP: Command = other(
    "fire_voltage_drop",
    "Fire Voltage Drop",
    "{source_v:number, segments:[{length_m?:number, ohm_per_km?:number, load_ma?:number, min_v?:number},:]} calculate a two-conductor radial DC line from source to end for one operating mode. Give installed one-way length, resistance per conductor at design temperature, local load and minimum device voltage",
    fire_voltage_drop,
);
pub(crate) const FIRE_LINE_LOAD: Command = other(
    "fire_line_load",
    "Fire Line Load",
    "{id:line id|GUID, loads?:[{id:device id|GUID, standby_ma?:number, fire_ma?:number},:], limit_ma?:number, reserve_percent?:number} sum device and isolator currents per mode. Use a documented common line limit and an explicit reserve (default zero). Missing currents are reported by device",
    fire_line_load,
);
pub(crate) const FIRE_BATTERY: Command = other(
    "fire_battery",
    "Fire Battery Capacity",
    "{standby_a?:number, fire_a?:number, source_a?:number, end_capacity_percent?:number, load_factor?:number, standby_load_factor?:number, fire_load_factor?:number, installed_ah?:number} size one backup battery bank under SP 6.13130.2025 Appendix B. Give equipment currents referred to the battery, excluding the source's own current. Use either one documented load factor (B.1) or both mode-specific factors (B.2). With installed capacity and positive fire current, report fire-only runtime (B.4). Missing capacity inputs are reported",
    fire_battery,
);
