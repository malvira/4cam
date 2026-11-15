//! Machine profiles: how a toolpath becomes G-code for a particular controller.

use serde::{Deserialize, Serialize};

/// How the controller interprets F on moves that mix linear axes with A.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedMode {
    /// F is along the vector (X, Z, A) with degrees counted as mm (GRBL / Smoothieware style).
    DegreesAsLength,
    /// F applies to the linear axes only; A just follows.
    LinearOnly,
    /// G93 inverse time: F = 1 / move time in minutes.
    InverseTime,
}

/// A machine/controller profile, normally loaded from `machines/*.json`.
///
/// Templates may use `{tool}`, `{rpm}` and `{safe}` placeholders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Machine {
    pub name: String,
    pub feed_mode: FeedMode,
    /// Decimals for axis words.
    #[serde(default = "default_decimals")]
    pub decimals: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_comment: Option<String>,
    #[serde(default)]
    pub preamble: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_rotary: Option<String>,
    pub tool_change: String,
    pub spindle_on: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dwell: Option<String>,
    #[serde(default)]
    pub postamble: Vec<String>,
}

fn default_decimals() -> usize {
    3
}

impl Machine {
    pub fn from_json(text: &str) -> Result<Machine, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// Built-in profile for the Makera Z1 with its 4th axis.
    pub fn makera_z1() -> Machine {
        Machine::from_json(include_str!("../../../machines/makera-z1.json"))
            .expect("bundled makera-z1.json is valid")
    }

    /// Built-in LinuxCNC profile (XYZA, G93 inverse-time feeds).
    pub fn linuxcnc() -> Machine {
        Machine::from_json(include_str!("../../../machines/linuxcnc.json"))
            .expect("bundled linuxcnc.json is valid")
    }

    /// Built-in profile by name: `makera-z1` or `linuxcnc`.
    pub fn builtin(name: &str) -> Option<Machine> {
        match name {
            "makera-z1" | "z1" => Some(Machine::makera_z1()),
            "linuxcnc" => Some(Machine::linuxcnc()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_z1_loads() {
        let m = Machine::makera_z1();
        assert_eq!(m.feed_mode, FeedMode::DegreesAsLength);
        assert_eq!(m.decimals, 3);
        assert_eq!(m.preamble, vec!["G21 G90 G94", "G17"]);
        assert_eq!(Machine::linuxcnc().feed_mode, FeedMode::InverseTime);
        assert!(Machine::builtin("nope").is_none());
    }
}
