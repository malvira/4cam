//! Job description: the JSON input format (schema version "0.1").

use serde::{Deserialize, Serialize};

/// The schema version this build reads and writes.
pub const SCHEMA_VERSION: &str = "0.1";

/// A complete CAM job: stock, part, tools and the programs to generate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Job {
    pub schema_version: String,
    pub stock: Stock,
    /// Radial clearance above the stock for safe rapids (mm).
    #[serde(default = "default_clearance")]
    pub clearance: f64,
    pub part: Part,
    pub tools: Vec<Tool>,
    pub spindle_rpm: u32,
    pub programs: Vec<Program>,
}

fn default_clearance() -> f64 {
    3.0
}

impl Job {
    /// Parse a job from JSON text.
    pub fn from_json(text: &str) -> Result<Job, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// Radius (Z) at which X and A rapids are safe.
    pub fn safe_r(&self) -> f64 {
        self.stock.radius + self.clearance
    }

    /// Look up a tool by number.
    pub fn tool(&self, number: u32) -> Option<&Tool> {
        self.tools.iter().find(|t| t.number == number)
    }
}

/// Round bar stock, held on the rotary axis.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Stock {
    pub radius: f64,
    pub length: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Part {
    pub profile: Profile,
}

/// Revolved profile: `[x, r]` points in ascending x. Two consecutive points with the same x
/// form a step; at the step itself the +x side wins.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    pub points: Vec<[f64; 2]>,
    /// Radius left past the last point (toward +x). Defaults to the last point's radius.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beyond_front: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Tool {
    pub number: u32,
    pub description: String,
    #[serde(flatten)]
    pub shape: ToolShape,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flute_length: Option<f64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolShape {
    /// Flat endmill.
    Flat { diameter: f64 },
    /// V-bit with the given included angle (degrees).
    Vbit { angle: f64 },
}

/// One output file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Program {
    pub name: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// Re-zero A at the start of the program.
    #[serde(default)]
    pub reset_a: bool,
    pub operations: Vec<Operation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Operation {
    RotaryTurning(RotaryTurning),
    HelicalKnurl(HelicalKnurl),
    FlankChamfer(FlankChamfer),
}

impl Operation {
    pub fn tool(&self) -> u32 {
        match self {
            Operation::RotaryTurning(o) => o.tool,
            Operation::HelicalKnurl(o) => o.tool,
            Operation::FlankChamfer(o) => o.tool,
        }
    }
}

/// Rough + finish the revolved profile with a flat endmill moving helically along X.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RotaryTurning {
    pub tool: u32,
    pub rough_depth: f64,
    pub rough_step: f64,
    pub rough_feed: f64,
    pub finish_allowance: f64,
    pub finish_step: f64,
    pub finish_feed: f64,
    pub plunge_feed: f64,
    #[serde(default = "default_segment_deg")]
    pub segment_deg: f64,
    #[serde(default = "default_grid")]
    pub grid: f64,
    #[serde(default = "default_footprint_step")]
    pub footprint_step: f64,
}

/// Diamond knurl: families of helical V grooves over a conical body.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HelicalKnurl {
    pub tool: u32,
    pub body_back: [f64; 2],
    pub body_front: [f64; 2],
    pub count: u32,
    pub helix_angle: f64,
    #[serde(default = "default_fill")]
    pub fill: f64,
    pub passes: Vec<f64>,
    #[serde(default = "default_families")]
    pub families: Vec<i32>,
    #[serde(default = "default_runout")]
    pub runout: f64,
    #[serde(default = "default_samples")]
    pub samples: usize,
    #[serde(default = "default_approach_clearance")]
    pub approach_clearance: f64,
    pub feed: f64,
    pub plunge_feed: f64,
}

/// Chamfer an edge with the flank of a V-bit while the part rotates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FlankChamfer {
    pub tool: u32,
    /// `[x, r]` of the chamfer's outer corner on the finished part.
    pub edge: [f64; 2],
    /// Axial distance from the edge to the V-bit tip.
    pub tip_offset: f64,
    /// Extra radial stock per pass (last pass normally 0).
    pub passes: Vec<f64>,
    #[serde(default = "default_revolutions")]
    pub revolutions: f64,
    #[serde(default = "default_segment_deg")]
    pub segment_deg: f64,
    pub feed: f64,
    pub plunge_feed: f64,
}

fn default_segment_deg() -> f64 {
    6.0
}
fn default_grid() -> f64 {
    0.02
}
fn default_footprint_step() -> f64 {
    0.01
}
fn default_fill() -> f64 {
    1.0
}
fn default_families() -> Vec<i32> {
    vec![1, -1]
}
fn default_runout() -> f64 {
    0.6
}
fn default_samples() -> usize {
    120
}
fn default_approach_clearance() -> f64 {
    0.5
}
fn default_revolutions() -> f64 {
    2.0
}
