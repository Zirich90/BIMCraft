use crate::engine::{Command, Session};
use crate::model::{MAX_POINTS, MAX_TEXT};
use serde_json::{Value, json};

use crate::commands::*;

fn rows<'a>(c: &str, p: &'a Value, key: &str) -> Result<&'a [Value], String> {
    p[key]
        .as_array()
        .filter(|values| (1..=MAX_POINTS).contains(&values.len()))
        .map(Vec::as_slice)
        .ok_or_else(|| bad(c, format!("{key} must have 1 to {MAX_POINTS} entries")))
}

fn positive(c: &str, p: &Value, key: &str) -> Result<f64, String> {
    num(c, p, key)?.filter(|value| *value > 0.0).ok_or_else(|| bad(c, format!("{key} must be positive")))
}

pub(crate) fn amplifier(_: &mut Session, p: &Value) -> R {
    const C: &str = "soue_amplifier_load";
    only(C, p, &["rated_w", "reserve_percent", "taps_w"])?;
    const MAX_WATTS: f64 = 1_000_000.0;
    let rated = positive(C, p, "rated_w")?;
    if rated > MAX_WATTS {
        return Err(bad(C, "rated_w must not exceed 1000000 W"));
    }
    let reserve = num(C, p, "reserve_percent")?.unwrap_or_default();
    if !(0.0..=100.0).contains(&reserve) {
        return Err(bad(C, "reserve_percent must be between 0 and 100"));
    }
    let allowed = rated * (1.0 - reserve / 100.0);
    let mut known = 0.0;
    let mut missing = Vec::new();
    for (i, tap) in rows(C, p, "taps_w")?.iter().enumerate() {
        if tap.is_null() {
            missing.push(i + 1);
        } else {
            known += tap
                .as_f64()
                .filter(|w| *w > 0.0 && *w <= MAX_WATTS)
                .ok_or_else(|| bad(C, format!("tap {} must be a positive bounded wattage", i + 1)))?;
        }
    }
    let status = if known - allowed > 1e-9 {
        "overload"
    } else if missing.is_empty() {
        "within_limit"
    } else {
        "missing_data"
    };
    Ok(json!({"known_w":known,"total_w":missing.is_empty().then_some(known),"allowed_w":allowed,"missing_taps":missing,"status":status}))
}

pub(crate) fn sound(_: &mut Session, p: &Value) -> R {
    const C: &str = "soue_sound_check";
    only(C, p, &["room", "class_f11", "design_max_dba", "points"])?;
    let (base, margin, maximum, sleeping): (f64, f64, f64, bool) = match p["room"].as_str() {
        Some("apartment") => (55.0, 10.0, 110.0, false),
        Some("other") => (65.0, 15.0, 110.0, false),
        Some("sleeping") => (75.0, 15.0, 95.0, true),
        _ => return Err(bad(C, "room must be apartment, other or sleeping")),
    };
    let maximum = if flag(C, p, "class_f11")? { maximum.min(95.0) } else { maximum };
    let maximum = match num(C, p, "design_max_dba")? {
        Some(limit) if limit > 0.0 && limit <= maximum => limit,
        Some(_) => return Err(bad(C, "design_max_dba must be positive and no greater than the normative maximum")),
        None => maximum,
    };
    let mut report = Vec::new();
    for (i, point) in rows(C, p, "points")?.iter().enumerate() {
        only(C, point, &["noise_dba", "signal_dba", "supplementary_alarm"])?;
        let level = |key| {
            let value = num(C, point, key)?;
            if value.is_some_and(|db| !(0.0..=200.0).contains(&db)) {
                return Err(bad(C, format!("point {}: {key} must be between 0 and 200 dBA", i + 1)));
            }
            Ok(value)
        };
        let noise = level("noise_dba")?;
        let signal = level("signal_dba")?;
        let supplementary = point
            .get("supplementary_alarm")
            .map(|value| value.as_bool().ok_or_else(|| bad(C, "supplementary_alarm must be boolean")))
            .transpose()?;
        let mut missing = Vec::new();
        let mut violations = Vec::new();
        let minimum = noise.map(|db| {
            let minimum = base.max(db + margin);
            if db > 85.0 && !sleeping {
                match supplementary {
                    Some(true) => {}
                    Some(false) => violations.push("supplementary_alarm_required"),
                    None => missing.push("supplementary_alarm"),
                }
                minimum.max(105.0)
            } else {
                minimum
            }
        });
        if noise.is_none() {
            missing.push("noise_dba");
        }
        if let Some(db) = signal {
            if db < minimum.unwrap_or(base) {
                violations.push("below_minimum");
            }
            if db > maximum {
                violations.push("above_maximum");
            }
        } else {
            missing.push("signal_dba");
        }
        report.push(json!({"point":i + 1,"minimum_dba":minimum,"maximum_dba":maximum,"missing":missing,"violations":violations}));
    }
    Ok(json!({"standard":"SP 3.13130.2026","points":report}))
}

pub(crate) fn parts(_: &mut Session, p: &Value) -> R {
    const C: &str = "fire_carrier_parts";
    only(C, p, &["runs", "bends", "tees", "unlicensed_workstations"])?;
    let mut supports = 0;
    let mut connectors = 0;
    for run in rows(C, p, "runs")? {
        only(C, run, &["length_m", "support_pitch_m", "section_m"])?;
        let length = positive(C, run, "length_m")?;
        let intervals = |pitch: f64| {
            let count = length / pitch;
            if pitch <= 0.0 || !count.is_finite() || count > 1_000_000.0 {
                return Err(bad(C, "pitch must be positive and yield at most 1000000 intervals"));
            }
            Ok(count.ceil().max(1.0) as u64)
        };
        supports += intervals(positive(C, run, "support_pitch_m")?)? + 1;
        if let Some(section) = num(C, run, "section_m")? {
            connectors += intervals(section)? - 1;
        }
    }
    let count = |key| {
        p.get(key)
            .map(|value| {
                value.as_u64().filter(|n| *n <= 1_000_000).ok_or_else(|| bad(C, format!("{key} must be an integer from 0 to 1000000")))
            })
            .transpose()
            .map(Option::unwrap_or_default)
    };
    Ok(json!({"supports":supports,"connectors":connectors,"bends":count("bends")?,"tees":count("tees")?,
        "software_licenses":count("unlicensed_workstations")?}))
}

pub(crate) fn okl(_: &mut Session, p: &Value) -> R {
    const C: &str = "fire_okl_check";
    const PARTS: [&str; 4] = ["cable", "carrier", "fastener", "mounting"];
    only(C, p, &["required_min", "installed", "tested", "profile"])?;
    let profile = super::systems::param(&p["profile"]).map_err(|e| bad(C, e))?;
    if profile.is_some_and(|profile| !profile.requires_okl()) && p.get("required_min").is_none() {
        return Ok(json!({"status":"optional","profile":profile}));
    }
    let required = positive(C, p, "required_min")?;
    let mut missing = Vec::new();
    let mut mismatches = Vec::new();
    let mut assemblies = [None; 2];
    let mut duration = None;
    let mut document = None;
    for (i, side) in ["installed", "tested"].into_iter().enumerate() {
        let Some(assembly) = p.get(side) else {
            missing.push(side.to_owned());
            continue;
        };
        let keys = if i == 0 {
            &["cable", "carrier", "fastener", "mounting", "pitch_mm"][..]
        } else {
            &["cable", "carrier", "fastener", "mounting", "pitch_mm", "duration_min", "document"][..]
        };
        only(C, assembly, keys)?;
        let text = |key: &str| {
            assembly
                .get(key)
                .map(|value| {
                    value
                        .as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty() && s.len() <= MAX_TEXT)
                        .ok_or_else(|| bad(C, format!("{side}.{key} must be bounded nonempty text")))
                })
                .transpose()
        };
        let mut parts = [None; PARTS.len()];
        for (index, key) in PARTS.into_iter().enumerate() {
            parts[index] = text(key)?;
            if parts[index].is_none() {
                missing.push(format!("{side}.{key}"));
            }
        }
        let pitch = num(C, assembly, "pitch_mm")?;
        match pitch {
            Some(value) if value <= 0.0 => return Err(bad(C, format!("{side}.pitch_mm must be positive"))),
            None => missing.push(format!("{side}.pitch_mm")),
            _ => {}
        }
        assemblies[i] = Some((parts, pitch));
        if i == 1 {
            duration = num(C, assembly, "duration_min")?;
            match duration {
                Some(value) if value <= 0.0 => return Err(bad(C, "tested.duration_min must be positive")),
                None => missing.push("tested.duration_min".to_owned()),
                _ => {}
            }
            document = text("document")?;
            if document.is_none() {
                missing.push("tested.document".to_owned());
            }
        }
    }
    if let [Some((installed, installed_pitch)), Some((tested, tested_pitch))] = assemblies {
        for (i, field) in PARTS.into_iter().enumerate() {
            if installed[i].zip(tested[i]).is_some_and(|(a, b)| a != b) {
                mismatches.push(field);
            }
        }
        if installed_pitch.zip(tested_pitch).is_some_and(|(a, b)| (a - b).abs() > 1e-6) {
            mismatches.push("pitch_mm");
        }
    }
    let insufficient = duration.is_some_and(|minutes| minutes < required);
    let status = if insufficient || !mismatches.is_empty() {
        "does_not_match"
    } else if missing.is_empty() {
        "matches_declared_evidence"
    } else {
        "missing_evidence"
    };
    Ok(json!({"required_min":required,"tested_min":duration,"document":document,"missing":missing,
        "mismatches":mismatches,"insufficient_duration":insufficient,"status":status}))
}
pub(crate) const SOUE_AMPLIFIER_LOAD: Command = other(
    "soue_amplifier_load",
    "Voice Alarm Amplifier Load",
    "{rated_w:number, reserve_percent?:number, taps_w:[number|null,:]} check one constant-voltage output. Use tap ratings at its output voltage and include all simultaneous loads; null identifies a missing rating",
    amplifier,
);
pub(crate) const SOUE_SOUND_CHECK: Command = other(
    "soue_sound_check",
    "Voice Alarm Sound Check",
    "{room:apartment|other|sleeping, class_f11?:bool, design_max_dba?:number, points:[{noise_dba?:number, signal_dba?:number, supplementary_alarm?:bool},:]} check supplied A-weighted levels with doors closed at 1.5 m above the floor, or at each sleeper's head. Supply effective noise during alarm",
    sound,
);
pub(crate) const FIRE_CARRIER_PARTS: Command = other(
    "fire_carrier_parts",
    "Carrier Parts Estimate",
    "{runs:[{length_m:number, support_pitch_m:number, section_m?:number},:], bends?:integer, tees?:integer, unlicensed_workstations?:integer} estimate supports including both ends, section connectors, bends, tees and software licences",
    parts,
);
pub(crate) const FIRE_OKL_CHECK: Command = other(
    "fire_okl_check",
    "Fire-Resistant Wiring Evidence",
    "{profile?:sps|soue_light|soue_sound|soue_voice|sot|skud|ots|mgn, required_min?:number, installed?:{cable:string, carrier:string, fastener:string, mounting:string, pitch_mm:number}, tested?:{cable:string, carrier:string, fastener:string, mounting:string, pitch_mm:number, duration_min:number, document:string}} compare one installed assembly with the declared documented test configuration. Use the design-required duration and report missing evidence",
    okl,
);
