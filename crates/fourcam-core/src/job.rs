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
    /// Revolved profile (used by `rotary_turning`, and as the finishing surface when no mesh
    /// is given).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<Profile>,
    /// Triangle mesh of the finished part (STL or 3MF).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh: Option<MeshSource>,
}

/// Where the part mesh comes from and how it sits on the rotary axis.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MeshSource {
    /// Path, relative to the job file.
    pub file: String,
    /// Mesh axis that becomes the rotary X axis.
    #[serde(default)]
    pub axis: crate::mesh::Axis,
    /// Translation applied after the axis mapping (mm).
    #[serde(default)]
    pub translate: [f64; 3],
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
    /// Ball endmill.
    Ball { diameter: f64 },
    /// V-bit with the given included angle (degrees). `diameter` (largest cutting diameter)
    /// is needed for mesh finishing.
    Vbit {
        angle: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diameter: Option<f64>,
    },
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
    RotaryFinish(RotaryFinish),
    RotaryRough(RotaryRough),
}

impl Operation {
    pub fn tool(&self) -> u32 {
        match self {
            Operation::RotaryFinish(o) => o.tool,
            Operation::RotaryRough(o) => o.tool,
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

/// Raster direction for [`RotaryFinish`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FinishPattern {
    /// Passes along X at constant A, stepping over in A (zig-zag, linked along A).
    Along,
    /// One helix around the axis, advancing `stepover_mm` in X per revolution.
    Around,
}

/// Finish the part mesh with a radial drop-cutter (flat, ball or V-bit).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RotaryFinish {
    pub tool: u32,
    pub pattern: FinishPattern,
    /// Axial extent `[start, end]` of the tool tip.
    pub x_range: [f64; 2],
    /// Angular extent (degrees) for `along`; a full turn by default.
    #[serde(default = "default_a_range")]
    pub a_range: [f64; 2],
    /// `along`: angular stepover (degrees). Alternatively give `stepover_mm`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stepover_deg: Option<f64>,
    /// `along`: stepover measured at the part's largest radius; `around`: X advance per turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stepover_mm: Option<f64>,
    /// Sample spacing along X for `along` passes (mm).
    #[serde(default = "default_sample_mm")]
    pub sample_mm: f64,
    /// Sample spacing around the axis (degrees) for `around` and for links.
    #[serde(default = "default_sample_deg")]
    pub sample_deg: f64,
    /// Radial stock to leave (mm).
    #[serde(default)]
    pub leave: f64,
    /// Largest allowed dip of a straight move below the surface (mm); moves are split until
    /// they stay within it.
    #[serde(default = "default_tolerance")]
    pub tolerance: f64,
    /// The tip never goes below this radius (mm, >= 0).
    #[serde(default)]
    pub min_radius: f64,
    /// `around`: add a full turn at the foot of every wall, which a helix would otherwise
    /// only touch once per turn.
    #[serde(default = "default_true")]
    pub clean_walls: bool,
    pub feed: f64,
    pub plunge_feed: f64,
}

fn default_true() -> bool {
    true
}

/// Rough the stock down to a mesh (or revolved profile) in radial levels, each a helix along X.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RotaryRough {
    pub tool: u32,
    /// Axial extent of the tool tip.
    pub x_range: [f64; 2],
    /// Radial depth per level (mm).
    pub step_down: f64,
    /// X advance per turn (mm).
    pub step_x: f64,
    /// Radial stock left on the part for finishing (mm).
    #[serde(default)]
    pub allowance: f64,
    /// Points per turn are spaced this far apart (degrees).
    #[serde(default = "default_rough_sample_deg")]
    pub sample_deg: f64,
    /// Largest dip of a straight move below the allowance surface (mm).
    #[serde(default = "default_tolerance")]
    pub tolerance: f64,
    /// The tip never goes below this radius (mm, >= 0).
    #[serde(default)]
    pub min_radius: f64,
    pub feed: f64,
    pub plunge_feed: f64,
}

fn default_rough_sample_deg() -> f64 {
    6.0
}

fn default_a_range() -> [f64; 2] {
    [0.0, 360.0]
}
fn default_tolerance() -> f64 {
    0.005
}
fn default_sample_mm() -> f64 {
    0.1
}
fn default_sample_deg() -> f64 {
    2.0
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
