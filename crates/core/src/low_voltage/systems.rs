//! Design profiles for explicitly classified low-voltage lines.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum System {
    Sps,
    SoueLight,
    SoueSound,
    SoueVoice,
    Sot,
    Skud,
    Ots,
    Mgn,
}

impl System {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Sps => "sps",
            Self::SoueLight => "soue_light",
            Self::SoueSound => "soue_sound",
            Self::SoueVoice => "soue_voice",
            Self::Sot => "sot",
            Self::Skud => "skud",
            Self::Ots => "ots",
            Self::Mgn => "mgn",
        }
    }

    pub const fn section(self) -> &'static str {
        match self {
            Self::SoueLight | Self::SoueSound | Self::SoueVoice => "soue",
            _ => self.code(),
        }
    }

    pub const fn mark_prefix(self) -> &'static str {
        match self {
            Self::Sps => "SPS-",
            Self::SoueLight => "SL-",
            Self::SoueSound => "SS-",
            Self::SoueVoice => "SV-",
            Self::Sot => "SOT-",
            Self::Skud => "SKUD-",
            Self::Ots => "OTS-",
            Self::Mgn => "MGN-",
        }
    }

    pub const fn requires_okl(self) -> bool {
        matches!(self, Self::Sps | Self::SoueLight | Self::SoueSound | Self::SoueVoice | Self::Mgn)
    }

    pub fn named(name: &str) -> Option<Self> {
        match name.trim().to_lowercase().as_str() {
            "sps" | "спс" => Some(Self::Sps),
            "soue_light" | "соуэ свет" => Some(Self::SoueLight),
            "soue_sound" | "соуэ звук" => Some(Self::SoueSound),
            "soue_voice" | "соуэ речь" => Some(Self::SoueVoice),
            "sot" | "сот" => Some(Self::Sot),
            "skud" | "скуд" => Some(Self::Skud),
            "ots" | "отс" => Some(Self::Ots),
            "mgn" | "мгн" => Some(Self::Mgn),
            _ => None,
        }
    }
}

pub(super) fn param(value: &serde_json::Value) -> Result<Option<System>, String> {
    serde_json::from_value(value.clone())
        .map_err(|_| "profile must be sps, soue_light, soue_sound, soue_voice, sot, skud, ots, mgn or null".into())
}

pub(crate) fn allows_compartment(systems: &[String], wire: &str, profile: Option<System>) -> bool {
    systems.is_empty()
        || match profile {
            Some(profile) => systems.iter().any(|name| {
                System::named(name) == Some(profile)
                    || section_for_name(name) == profile.section() && profile.section() == "soue" && System::named(name).is_none()
            }),
            None => {
                systems.iter().any(|name| name == wire)
                    || System::named(wire).is_some_and(|profile| allows_compartment(systems, wire, Some(profile)))
            }
        }
}

pub(super) fn section_for_name(name: &str) -> &str {
    if name.eq_ignore_ascii_case("soue") || name.trim().to_lowercase() == "соуэ" {
        "soue"
    } else {
        match System::named(name) {
            Some(profile) => profile.section(),
            None => name,
        }
    }
}
