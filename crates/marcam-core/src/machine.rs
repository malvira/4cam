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
    /// F is mm/min along the move, the rotation counted as its arc at the tool's radius as
    /// the controller estimates it: the distance of the work Y, Z position from the axis plus
    /// [`Machine::surface_radius_pad`]. Pure linear moves are plain mm/min. This is how
    /// Makera's Carvera firmware reads F on moves with A (see docs/profiles.md).
    SurfaceSpeed,
}

/// The distance the controller divides by F (mm/min) for a move of `d` = (X, Y, Z mm, A deg)
/// with the tool `radius` mm from the rotary axis (mean over the move); `pad` is the
/// surface-speed radius pad. Inverse time has no such distance (1).
pub fn feed_length(mode: FeedMode, pad: f64, d: [f64; 4], radius: f64) -> f64 {
    let lin = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    match mode {
        FeedMode::DegreesAsLength => (lin * lin + d[3] * d[3]).sqrt(),
        FeedMode::LinearOnly => {
            if lin > 1e-9 {
                lin
            } else {
                d[3].abs()
            }
        }
        FeedMode::InverseTime => 1.0,
        FeedMode::SurfaceSpeed => {
            let arc = (radius.abs() + pad) * d[3].to_radians().abs();
            (lin * lin + arc * arc).sqrt()
        }
    }
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

/// A machine/controller profile, normally loaded from `machines/*.json`.
///
/// Templates may use `{tool}`, `{rpm}` and `{safe}` placeholders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Machine {
    pub name: String,
    pub feed_mode: FeedMode,
    /// `surface_speed` only: mm the controller adds to the tool's distance from the axis when
    /// it turns F into a rotation rate (stock Carvera firmware: 15, i.e. 30 mm on the diameter).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub surface_radius_pad: f64,
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
    /// The controller runs drilling canned cycles (G81, G83, G80, G98): a drill operation that
    /// asks for one is written so.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub canned_cycles: bool,
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
