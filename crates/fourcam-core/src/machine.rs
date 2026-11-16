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
    /// How the toolpath's X, Y, Z and A become this machine's words, from its kinematics
    /// (filled in when a profile is loaded; identity for the usual XYZ + A about +X).
    #[serde(default, skip_serializing_if = "AxisMap::is_identity")]
    pub axes: AxisMap,
}

/// The toolpath works in one convention: X, Y, Z are the tool's motion relative to the work,
/// and A turns the workpiece right-handed about +X. A machine's word for each of them, and
/// the sign that turns the toolpath's value into the word's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AxisMap {
    pub letters: [String; 4],
    pub signs: [f64; 4],
}

impl Default for AxisMap {
    fn default() -> Self {
        AxisMap {
            letters: ["X", "Y", "Z", "A"].map(String::from),
            signs: [1.0; 4],
        }
    }
}

impl AxisMap {
    pub fn is_identity(&self) -> bool {
        *self == AxisMap::default()
    }

    /// The word for toolpath axis `k` (0 X, 1 Y, 2 Z, 3 A) at value `v`, with `d` decimals.
    pub fn word(&self, k: usize, v: f64, d: usize) -> String {
        let v = self.signs[k] * v;
        format!("{}{v:.d$}", self.letters[k])
    }
}

fn default_decimals() -> usize {
    3
}

impl Machine {
    pub fn from_json(text: &str) -> Result<Machine, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// Built-in post for the Makera Z1 with its 4th axis.
    pub fn makera_z1() -> Machine {
        Machine::builtin("makera-z1-4axis").expect("built in")
    }

    /// Built-in LinuxCNC post (XYZA, G93 inverse-time feeds).
    pub fn linuxcnc() -> Machine {
        Machine::builtin("linuxcnc-4axis").expect("built in")
    }

    /// Post templates of a built-in machine profile (see [`crate::profiles`]), by id or alias
    /// (`makera-z1`, `linuxcnc`, `carvera`, `carvera-air`).
    pub fn builtin(name: &str) -> Option<Machine> {
        crate::profiles::builtin_machine(name).map(|p| p.post)
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
