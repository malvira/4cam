//! Job description: the JSON input format (schema version "0.1").

use serde::{Deserialize, Serialize};

/// The schema version this build reads and writes.
pub const SCHEMA_VERSION: &str = "0.1";

/// A complete CAM job: stock, part, tools and the programs to generate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Job {
    pub schema_version: String,
    /// Machine profile id (see `profiles/`), used when no machine is given explicitly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    pub stock: Stock,
    /// Box stock for 3-axis (vise) setups, in work coordinates (Z up).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stock_box: Option<StockBox>,
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

/// Rectangular block, held in a vise: corners in work coordinates (Z up).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct StockBox {
    pub min: [f64; 3],
    pub max: [f64; 3],
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
    /// The file is a grayscale image (PNG or PGM) to build a relief from, not a mesh.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<crate::relief::ImageRelief>,
}

impl MeshSource {
    /// The part from the file's bytes, placed as this source says: an STL/3MF mesh, or the
    /// relief built from an image; then the axis mapping and translation.
    pub fn mesh(&self, bytes: &[u8]) -> Result<crate::Mesh, crate::mesh::MeshError> {
        let m = match &self.image {
            Some(r) => crate::relief::relief_mesh(&crate::relief::decode_gray(bytes)?, r),
            None => crate::Mesh::from_bytes(bytes)?,
        };
        Ok(m.oriented(self.axis, self.translate))
    }

    /// [`MeshSource::mesh`] from the file, relative to `dir`.
    pub fn load(&self, dir: &std::path::Path) -> Result<crate::Mesh, crate::mesh::MeshError> {
        let path = dir.join(&self.file);
        let data = std::fs::read(&path)
            .map_err(|e| crate::mesh::MeshError::Io(format!("{}: {e}", path.display())))?;
        self.mesh(&data).map_err(|e| match e {
            crate::mesh::MeshError::Format(s) => {
                crate::mesh::MeshError::Format(format!("{}: {s}", path.display()))
            }
            other => other,
        })
    }
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
    /// Shank diameter above the flutes (default: the cutting diameter).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shank_diameter: Option<f64>,
    /// Length from the tip to the holder (default: flute length + 5 mm).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stickout: Option<f64>,
    /// Holder (collet nut) diameter (default 20 mm).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder_diameter: Option<f64>,
    /// Holder length above the stickout (default 30 mm).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder_length: Option<f64>,
}

impl Tool {
    /// A tool with only its number, description and shape given (everything else default).
    pub fn new(number: u32, description: &str, shape: ToolShape) -> Tool {
        Tool {
            number,
            description: description.into(),
            shape,
            flute_length: None,
            shank_diameter: None,
            stickout: None,
            holder_diameter: None,
            holder_length: None,
        }
    }
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
    /// Tapered ball end mill: a ball of `tip_diameter` blending into a cone whose sides are
    /// `taper_angle` (deg) from the axis, up to `diameter`. A taper of 0 is a ball end mill
    /// of the tip's size.
    TaperedBall {
        tip_diameter: f64,
        taper_angle: f64,
        diameter: f64,
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
    Face(Face),
    Pocket(Pocket),
    Drill(Drill),
    Bore(Bore),
    Contour(Contour),
    Raster3d(Raster3d),
    /// 3D roughing: the parallel raster with a flat (or ball) tool in step-down levels,
    /// leaving stock for the finish.
    Rough3d(Raster3d),
    Engrave(Engrave),
    Vcarve(Vcarve),
}

/// 3-axis: face the top of the stock down by `depth`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Face {
    pub tool: u32,
    /// Rotary setups: the A angle (deg) the part is indexed to before this 3-axis
    /// operation (indexed 3+1). Vise setups: must be 0.
    #[serde(default)]
    pub index_a: f64,
    pub depth: f64,
    pub step_down: f64,
    /// Stepover as a fraction of the tool diameter.
    pub stepover: f64,
    pub feed: f64,
    pub plunge_feed: f64,
}

/// 3-axis: rectangular pocket `[x0, y0, x1, y1]`, `depth` below `top` (default: stock top).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Pocket {
    pub tool: u32,
    /// Rotary setups: the A angle (deg) the part is indexed to before this 3-axis
    /// operation (indexed 3+1). Vise setups: must be 0.
    #[serde(default)]
    pub index_a: f64,
    /// Rectangle `[x0, y0, x1, y1]`, used when `paths` is empty.
    #[serde(default)]
    pub rect: [f64; 4],
    /// Closed outlines (even-odd: an outline and the islands inside it, any shape), cleared
    /// by offsets instead of the rectangle when given.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<Vec<[f64; 2]>>,
    pub depth: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top: Option<f64>,
    pub step_down: f64,
    /// Fraction of the tool diameter.
    pub stepover: f64,
    pub feed: f64,
    pub plunge_feed: f64,
}

/// 3-axis: holes at `points`, pecking `peck` at a time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Drill {
    pub tool: u32,
    /// Rotary setups: the A angle (deg) the part is indexed to before this 3-axis
    /// operation (indexed 3+1). Vise setups: must be 0.
    #[serde(default)]
    pub index_a: f64,
    pub points: Vec<[f64; 2]>,
    pub depth: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top: Option<f64>,
    pub peck: f64,
    /// Clearance above the top for retracts between pecks.
    pub retract: f64,
    pub feed: f64,
}

/// 3-axis: bore round holes at `points` to `diameter` with an end mill smaller than them:
/// helical entry, roughing rings out to the finishing allowance, the finish at the size and
/// the spring passes; G2/G3 arcs (`arcs`) or straight segments within `tolerance`. See
/// [`crate::strategy::bore`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Bore {
    pub tool: u32,
    /// Rotary setups: the A angle (deg) the part is indexed to (indexed 3+1, a radial hole).
    #[serde(default)]
    pub index_a: f64,
    pub points: Vec<[f64; 2]>,
    pub diameter: f64,
    pub depth: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top: Option<f64>,
    /// Descent per turn of the helix (mm).
    #[serde(default = "default_helix_pitch")]
    pub helix_pitch: f64,
    /// Roughing rings at most this far apart, as a fraction of the tool diameter.
    #[serde(default = "default_bore_stepover")]
    pub stepover: f64,
    /// Radial stock the roughing leaves for the finish (mm).
    #[serde(default = "default_bore_allowance")]
    pub finish_allowance: f64,
    /// Finishing passes repeated at the size (for the tool's deflection).
    #[serde(default = "default_spring_passes")]
    pub spring_passes: u32,
    /// "climb" (counter-clockwise inside the hole, G3) or "conventional" (G2).
    #[serde(default = "default_direction")]
    pub direction: String,
    /// Write the circles as G2/G3 arcs (else straight segments).
    #[serde(default = "default_true")]
    pub arcs: bool,
    /// Largest distance between the segments and the circle (mm).
    #[serde(default = "default_bore_tolerance")]
    pub tolerance: f64,
    pub feed: f64,
    pub plunge_feed: f64,
    /// Descents gentler than this (degrees below the horizontal, along the path) run at
    /// the cutting feed; steeper ones are slowed so the tip falls no faster than
    /// `plunge_feed`. 0: every descent is slowed.
    #[serde(default = "default_ramp_angle")]
    pub ramp_angle: f64,
}

fn default_helix_pitch() -> f64 {
    0.5
}
fn default_bore_stepover() -> f64 {
    0.4
}
fn default_bore_allowance() -> f64 {
    0.1
}
fn default_spring_passes() -> u32 {
    1
}
fn default_bore_tolerance() -> f64 {
    0.002
}

/// 3-axis: follow a closed 2D path, outside / inside / on it, in step-down passes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Contour {
    pub tool: u32,
    /// Rotary setups: the A angle (deg) the part is indexed to before this 3-axis
    /// operation (indexed 3+1). Vise setups: must be 0.
    #[serde(default)]
    pub index_a: f64,
    pub path: Vec<[f64; 2]>,
    /// "outside", "inside" or "on".
    pub side: String,
    pub depth: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top: Option<f64>,
    pub step_down: f64,
    pub feed: f64,
    pub plunge_feed: f64,
}

/// 3-axis: drop-cutter raster over the part mesh (passes along X, stepping in Y), cut in
/// step-down levels; `floor` is where there is no part.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Raster3d {
    pub tool: u32,
    /// Rotary setups: the A angle (deg) the part is indexed to before this 3-axis
    /// operation (indexed 3+1). Vise setups: must be 0.
    #[serde(default)]
    pub index_a: f64,
    pub x_range: [f64; 2],
    pub y_range: [f64; 2],
    pub stepover: f64,
    pub sample: f64,
    pub step_down: f64,
    pub floor: f64,
    #[serde(default)]
    pub leave: f64,
    #[serde(default = "default_tolerance")]
    pub tolerance: f64,
    /// Pass direction, degrees from +X (0: passes along X, stepping in Y).
    #[serde(default)]
    pub angle: f64,
    /// "zigzag" (back and forth, linked at depth) or "one_way" (every pass the same way,
    /// re-entering from above the stock).
    #[serde(default = "default_pattern")]
    pub pattern: String,
    /// One-way passes: "climb" or "conventional" (M3 / clockwise spindle).
    #[serde(default = "default_direction")]
    pub direction: String,
    /// Target scallop height (mm). When > 0 it sets the stepover from the tool shape
    /// (ball or V-bit); `stepover` is used otherwise.
    #[serde(default)]
    pub scallop: f64,
    /// Face regions of the part mesh the tool may touch (ids from
    /// [`crate::geometry::face_regions`] at 30°, filled from a face selection): passes run
    /// only where the tool rests on them, following the whole part. Empty: the whole part.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub faces: Vec<String>,
    pub feed: f64,
    pub plunge_feed: f64,
    /// Descents gentler than this (degrees below the horizontal, along the path) run at
    /// the cutting feed; steeper ones are slowed so the tip falls no faster than
    /// `plunge_feed`. 0: every descent is slowed.
    #[serde(default = "default_ramp_angle")]
    pub ramp_angle: f64,
}

/// Follow paths and single-line text at `depth` below the surface, in `step_down` passes:
/// on a vise at Z `top`, or wrapped round the rotary axis at radius `top` (path Y = arc
/// length, centred on the top of the bar at A0).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Engrave {
    pub tool: u32,
    /// Polylines (closed when the last point repeats the first).
    #[serde(default)]
    pub paths: Vec<Vec<[f64; 2]>>,
    /// Text in the built-in single-stroke font (A-Z, 0-9, - . , / : ! ? + = ( ) ').
    #[serde(default)]
    pub text: String,
    /// Cap height of the text (mm).
    #[serde(default = "default_text_height")]
    pub text_height: f64,
    /// Start of the text's first baseline.
    #[serde(default)]
    pub text_at: [f64; 2],
    /// Turn the text by this many degrees about `text_at` (90: it runs along +Y, i.e.
    /// round the bar when wrapped).
    #[serde(default)]
    pub text_rotate: f64,
    pub depth: f64,
    /// Surface: Z on a vise (default: the stock top), radius on a rotary setup (default:
    /// the stock radius).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top: Option<f64>,
    pub step_down: f64,
    pub feed: f64,
    pub plunge_feed: f64,
    /// Descents gentler than this (degrees below the horizontal, along the path) run at
    /// the cutting feed; steeper ones are slowed so the tip falls no faster than
    /// `plunge_feed`. 0: every descent is slowed.
    #[serde(default = "default_ramp_angle")]
    pub ramp_angle: f64,
}

fn default_text_weight() -> f64 {
    crate::engrave::OUTLINE_WEIGHT
}

fn default_text_height() -> f64 {
    5.0
}

/// V-carve closed paths (even-odd fill) with a V-bit: depth from the bit's angle and the
/// distance to the outline, up to `max_depth`; flat or wrapped as [`Engrave`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Vcarve {
    pub tool: u32,
    /// Closed outlines (even-odd).
    #[serde(default)]
    pub paths: Vec<Vec<[f64; 2]>>,
    /// Text in outline letters (marcam's own font, see [`crate::engrave::outline_text`]),
    /// carved with the paths.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// Cap height of the text (mm).
    #[serde(default = "default_text_height")]
    pub text_height: f64,
    /// Start of the text's first baseline.
    #[serde(default)]
    pub text_at: [f64; 2],
    /// Turn the text by this many degrees about `text_at`.
    #[serde(default)]
    pub text_rotate: f64,
    /// Letter weight: the pen's width as a fraction of the cap height.
    #[serde(default = "default_text_weight")]
    pub text_weight: f64,
    pub max_depth: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top: Option<f64>,
    /// Distance between rows (mm).
    pub stepover: f64,
    /// Point spacing along a row (mm).
    pub sample: f64,
    #[serde(default = "default_tolerance")]
    pub tolerance: f64,
    pub feed: f64,
    pub plunge_feed: f64,
    /// Descents gentler than this (degrees below the horizontal, along the path) run at
    /// the cutting feed; steeper ones are slowed so the tip falls no faster than
    /// `plunge_feed`. 0: every descent is slowed.
    #[serde(default = "default_ramp_angle")]
    pub ramp_angle: f64,
}

fn default_pattern() -> String {
    "zigzag".into()
}

fn default_direction() -> String {
    "climb".into()
}

impl Operation {
    /// A angle a 3-axis operation is indexed to (None for rotary operations).
    pub fn index_a(&self) -> Option<f64> {
        match self {
            Operation::Face(o) => Some(o.index_a),
            Operation::Pocket(o) => Some(o.index_a),
            Operation::Drill(o) => Some(o.index_a),
            Operation::Bore(o) => Some(o.index_a),
            Operation::Contour(o) => Some(o.index_a),
            Operation::Raster3d(o) | Operation::Rough3d(o) => Some(o.index_a),
            _ => None,
        }
    }

    /// The ramp angle of an operation that descends along its path (None: plunges only).
    pub fn ramp_angle(&self) -> Option<f64> {
        match self {
            Operation::Raster3d(o) | Operation::Rough3d(o) => Some(o.ramp_angle),
            Operation::Engrave(o) => Some(o.ramp_angle),
            Operation::Vcarve(o) => Some(o.ramp_angle),
            Operation::RotaryTurning(o) => Some(o.ramp_angle),
            Operation::HelicalKnurl(o) => Some(o.ramp_angle),
            Operation::FlankChamfer(o) => Some(o.ramp_angle),
            Operation::RotaryFinish(o) => Some(o.ramp_angle),
            Operation::RotaryRough(o) => Some(o.ramp_angle),
            Operation::Bore(o) => Some(o.ramp_angle),
            _ => None,
        }
    }

    pub fn tool(&self) -> u32 {
        match self {
            Operation::RotaryFinish(o) => o.tool,
            Operation::RotaryRough(o) => o.tool,
            Operation::Face(o) => o.tool,
            Operation::Pocket(o) => o.tool,
            Operation::Drill(o) => o.tool,
            Operation::Bore(o) => o.tool,
            Operation::Contour(o) => o.tool,
            Operation::Raster3d(o) | Operation::Rough3d(o) => o.tool,
            Operation::Engrave(o) => o.tool,
            Operation::Vcarve(o) => o.tool,
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
    /// Descents gentler than this (degrees below the horizontal, along the path) run at
    /// the cutting feed; steeper ones are slowed so the tip falls no faster than
    /// `plunge_feed`. 0: every descent is slowed.
    #[serde(default = "default_ramp_angle")]
    pub ramp_angle: f64,
    #[serde(default = "default_segment_deg")]
    pub segment_deg: f64,
    #[serde(default = "default_grid")]
    pub grid: f64,
    #[serde(default = "default_footprint_step")]
    pub footprint_step: f64,
    /// Turn only this X span of the profile (picked bands); None: all of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_range: Option<[f64; 2]>,
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
    /// Descents gentler than this (degrees below the horizontal, along the path) run at
    /// the cutting feed; steeper ones are slowed so the tip falls no faster than
    /// `plunge_feed`. 0: every descent is slowed.
    #[serde(default = "default_ramp_angle")]
    pub ramp_angle: f64,
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
    /// Descents gentler than this (degrees below the horizontal, along the path) run at
    /// the cutting feed; steeper ones are slowed so the tip falls no faster than
    /// `plunge_feed`. 0: every descent is slowed.
    #[serde(default = "default_ramp_angle")]
    pub ramp_angle: f64,
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
    /// Face regions of the part mesh the tool may touch (ids from
    /// [`crate::geometry::face_regions`] at 30°, filled from a face selection): passes run
    /// only where the tool rests on them, following the whole part. Empty: the whole part.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub faces: Vec<String>,

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
    /// Descents gentler than this (degrees below the horizontal, along the path) run at
    /// the cutting feed; steeper ones are slowed so the tip falls no faster than
    /// `plunge_feed`. 0: every descent is slowed.
    #[serde(default = "default_ramp_angle")]
    pub ramp_angle: f64,
}

/// Descents at up to 30 degrees run at the cutting feed (Mar, 2026-11-17).
pub fn default_ramp_angle() -> f64 {
    30.0
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
    /// Descents gentler than this (degrees below the horizontal, along the path) run at
    /// the cutting feed; steeper ones are slowed so the tip falls no faster than
    /// `plunge_feed`. 0: every descent is slowed.
    #[serde(default = "default_ramp_angle")]
    pub ramp_angle: f64,
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
