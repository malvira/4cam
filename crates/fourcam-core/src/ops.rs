//! Operation registry: each operation type describes its parameters (with units, defaults and
//! limits), the tool shapes and geometry it accepts, and the machine abilities it needs. Hosts
//! (CLI, web app) read it to build forms and validate operations; planning dispatches on the
//! same `type` names.

use crate::job::{Operation, ToolShape};
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ParamSpec {
    pub name: &'static str,
    /// "number", "integer", "boolean", "vec2" ([a, b]), "rect" ([x0, y0, x1, y1]), "numbers"
    /// (list), "integers", "points" ([[x, y], ...]), "choice"
    pub kind: &'static str,
    #[serde(skip_serializing_if = "str::is_empty")]
    pub unit: &'static str,
    pub required: bool,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub default: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pub choices: &'static [&'static str],
    pub help: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct OperationType {
    pub kind: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    /// "roughing", "finishing", "feature"
    pub group: &'static str,
    pub params: Vec<ParamSpec>,
    /// Tool kinds it can use ("flat", "ball", "vbit").
    pub tool_shapes: &'static [&'static str],
    /// Geometry it works from: "profile", "mesh", "axial_range", "edge".
    pub geometry: &'static [&'static str],
    /// Machine abilities needed: "rotary_a" (A about X).
    pub machine_needs: &'static [&'static str],
    /// Whether it may cut below the part surface on purpose (knurls, chamfers).
    pub shapes_part: bool,
}

const fn p(
    name: &'static str,
    kind: &'static str,
    unit: &'static str,
    help: &'static str,
) -> ParamSpec {
    ParamSpec {
        name,
        kind,
        unit,
        required: true,
        default: Value::Null,
        min: None,
        max: None,
        choices: &[],
        help,
    }
}

impl ParamSpec {
    fn opt(mut self, default: Value) -> Self {
        self.required = false;
        self.default = default;
        self
    }
    fn min(mut self, v: f64) -> Self {
        self.min = Some(v);
        self
    }
    fn max(mut self, v: f64) -> Self {
        self.max = Some(v);
        self
    }
    fn choices(mut self, c: &'static [&'static str]) -> Self {
        self.choices = c;
        self
    }
}

fn tool() -> ParamSpec {
    p("tool", "integer", "", "Tool number (T).").min(1.0)
}
fn feed() -> ParamSpec {
    p(
        "feed",
        "number",
        "mm/min",
        "Cutting feed along the surface.",
    )
    .min(0.001)
}
fn plunge() -> ParamSpec {
    p(
        "plunge_feed",
        "number",
        "mm/min",
        "Fastest the tip may descend.",
    )
    .min(0.001)
}

/// Every operation type 4cam can plan.
pub fn registry() -> Vec<OperationType> {
    vec![
        OperationType {
            kind: "rotary_turning",
            title: "Rotary turning",
            description: "Rough the bar to a revolved profile in radial levels, then finish it with one helical pass (flat end mill).",
            group: "roughing",
            params: vec![
                tool(),
                p("rough_depth", "number", "mm", "Radial depth per roughing level.").min(0.01),
                p("rough_step", "number", "mm/rev", "X advance per turn while roughing.").min(0.01),
                p("rough_feed", "number", "mm/min", "Roughing feed.").min(0.001),
                p("finish_allowance", "number", "mm", "Radial stock left by roughing.").min(0.0),
                p("finish_step", "number", "mm/rev", "X advance per turn while finishing.").min(0.01),
                p("finish_feed", "number", "mm/min", "Finishing feed.").min(0.001),
                plunge(),
                p("segment_deg", "number", "deg", "Angular length of each rotary move.").opt(json!(6.0)).min(0.1).max(90.0),
                p("grid", "number", "mm", "X spacing of the tool-radius table.").opt(json!(0.02)).min(0.001),
                p("footprint_step", "number", "mm", "Sampling of the flat tool's footprint.").opt(json!(0.01)).min(0.001),
            ],
            tool_shapes: &["flat"],
            geometry: &["profile"],
            machine_needs: &["rotary_a"],
            shapes_part: false,
        },
        OperationType {
            kind: "rotary_rough",
            title: "Rotary roughing (mesh)",
            description: "Rough the bar to the part (mesh or profile) plus an allowance, in radial levels, each a helix along X.",
            group: "roughing",
            params: vec![
                tool(),
                p("x_range", "vec2", "mm", "Axial extent of the tool tip [start, end].").min(-10000.0),
                p("step_down", "number", "mm", "Radial depth per level.").min(0.01),
                p("step_x", "number", "mm/rev", "X advance per turn.").min(0.01),
                p("allowance", "number", "mm", "Radial stock left for finishing.").opt(json!(0.0)).min(0.0),
                p("sample_deg", "number", "deg", "Point spacing around the axis.").opt(json!(6.0)).min(0.1),
                p("tolerance", "number", "mm", "Largest dip of a straight move below the allowance surface.").opt(json!(0.005)).min(0.0001),
                p("min_radius", "number", "mm", "The tip never goes below this radius.").opt(json!(0.0)).min(0.0),
                feed(),
                plunge(),
            ],
            tool_shapes: &["flat", "ball", "vbit"],
            geometry: &["mesh", "profile", "axial_range"],
            machine_needs: &["rotary_a"],
            shapes_part: false,
        },
        OperationType {
            kind: "rotary_finish",
            title: "Rotary finishing (mesh)",
            description: "Finish the part with a radial drop-cutter: rasters along X at constant A, or a helix around X.",
            group: "finishing",
            params: vec![
                tool(),
                p("pattern", "choice", "", "Rasters along X, or one helix around X.").choices(&["along", "around"]),
                p("x_range", "vec2", "mm", "Axial extent of the tool tip [start, end]."),
                p("a_range", "vec2", "deg", "Angular extent for 'along'.").opt(json!([0.0, 360.0])),
                p("stepover_deg", "number", "deg", "'along': angular stepover.").opt(Value::Null).min(0.01),
                p("stepover_mm", "number", "mm", "'along': stepover at the largest radius; 'around': X advance per turn.").opt(Value::Null).min(0.001),
                p("sample_mm", "number", "mm", "Point spacing along X ('along').").opt(json!(0.1)).min(0.001),
                p("sample_deg", "number", "deg", "Point spacing around X ('around' and links).").opt(json!(2.0)).min(0.01),
                p("leave", "number", "mm", "Radial stock to leave.").opt(json!(0.0)),
                p("tolerance", "number", "mm", "Largest dip of a straight move below the surface.").opt(json!(0.005)).min(0.0001),
                p("min_radius", "number", "mm", "The tip never goes below this radius.").opt(json!(0.0)).min(0.0),
                p("clean_walls", "boolean", "", "'around': a full turn at the foot of each wall.").opt(json!(true)),
                feed(),
                plunge(),
            ],
            tool_shapes: &["flat", "ball", "vbit"],
            geometry: &["mesh", "profile", "axial_range"],
            machine_needs: &["rotary_a"],
            shapes_part: false,
        },
        OperationType {
            kind: "helical_knurl",
            title: "Helical (diamond) knurl",
            description: "Families of helical V grooves over a conical body; two opposite families make diamonds.",
            group: "feature",
            params: vec![
                tool(),
                p("body_back", "vec2", "mm", "[x, r] of the body's back edge."),
                p("body_front", "vec2", "mm", "[x, r] of the body's front edge."),
                p("count", "integer", "", "Grooves per family (diamonds around).").min(1.0),
                p("helix_angle", "number", "deg", "Groove angle from the X axis.").min(0.0).max(89.0),
                p("fill", "number", "", "1 = full-point pyramids; less leaves flats on the peaks.").opt(json!(1.0)).min(0.0).max(1.0),
                p("passes", "numbers", "", "Fraction of the groove depth per pass, e.g. [0.6, 1.0]."),
                p("families", "integers", "", "Helix directions: [1, -1] for diamonds.").opt(json!([1, -1])),
                p("runout", "number", "mm", "How far grooves run past both ends.").opt(json!(0.6)).min(0.0),
                p("samples", "integer", "", "Points along each groove.").opt(json!(120)).min(2.0),
                p("approach_clearance", "number", "mm", "Rapid down to this far above the body.").opt(json!(0.5)).min(0.0),
                feed(),
                plunge(),
            ],
            tool_shapes: &["vbit"],
            geometry: &["axial_range"],
            machine_needs: &["rotary_a"],
            shapes_part: true,
        },
        OperationType {
            kind: "flank_chamfer",
            title: "V-bit flank chamfer",
            description: "Chamfer an edge with the flank of a V-bit while the part turns.",
            group: "feature",
            params: vec![
                tool(),
                p("edge", "vec2", "mm", "[x, r] of the chamfer's outer corner."),
                p("tip_offset", "number", "mm", "Axial distance from the edge to the V-bit tip."),
                p("passes", "numbers", "mm", "Extra radial stock per pass (last normally 0)."),
                p("revolutions", "number", "", "Turns per pass.").opt(json!(2.0)).min(0.1),
                p("segment_deg", "number", "deg", "Angular length of each move.").opt(json!(6.0)).min(0.1),
                feed(),
                plunge(),
            ],
            tool_shapes: &["vbit"],
            geometry: &["edge"],
            machine_needs: &["rotary_a"],
            shapes_part: true,
        },
        OperationType {
            kind: "face",
            title: "Facing (3-axis)",
            description: "Face the top of the block down by a depth, in zig-zag passes along X.",
            group: "roughing",
            params: vec![
                tool(),
                p("index_a", "number", "deg", "Rotary setups: the A angle the part is turned to first (indexed 3+1); 0 on a vise.").opt(json!(0.0)),
                p("depth", "number", "mm", "How much to take off the top.").min(0.0),
                p("step_down", "number", "mm", "Depth per pass.").min(0.01),
                p("stepover", "number", "", "Stepover as a fraction of the tool diameter.").min(0.05).max(1.0),
                feed(),
                plunge(),
            ],
            tool_shapes: &["flat"],
            geometry: &["stock"],
            machine_needs: &["xyz"],
            shapes_part: false,
        },
        OperationType {
            kind: "pocket",
            title: "Rectangular pocket (3-axis)",
            description: "Clear a rectangle to a depth in zig-zag levels, then finish its walls.",
            group: "roughing",
            params: vec![
                tool(),
                p("index_a", "number", "deg", "Rotary setups: the A angle the part is turned to first (indexed 3+1); 0 on a vise.").opt(json!(0.0)),
                p("rect", "rect", "mm", "[x0, y0, x1, y1] of the pocket."),
                p("depth", "number", "mm", "Depth below the top.").min(0.01),
                p("top", "number", "mm", "Top of the pocket (default: stock top).").opt(Value::Null),
                p("step_down", "number", "mm", "Depth per level.").min(0.01),
                p("stepover", "number", "", "Stepover as a fraction of the tool diameter.").min(0.05).max(1.0),
                feed(),
                plunge(),
            ],
            tool_shapes: &["flat"],
            geometry: &["rectangle"],
            machine_needs: &["xyz"],
            shapes_part: false,
        },
        OperationType {
            kind: "drill",
            title: "Drilling / pecking (3-axis)",
            description: "Peck holes at points with an end mill.",
            group: "feature",
            params: vec![
                tool(),
                p("index_a", "number", "deg", "Rotary setups: the A angle the part is turned to first (indexed 3+1); 0 on a vise.").opt(json!(0.0)),
                p("points", "points", "mm", "Hole centres as [[x, y], ...]."),
                p("depth", "number", "mm", "Hole depth below the top.").min(0.01),
                p("top", "number", "mm", "Top (default: stock top).").opt(Value::Null),
                p("peck", "number", "mm", "Depth per peck.").min(0.01),
                p("retract", "number", "mm", "Clearance above the top between pecks.").min(0.0),
                feed(),
            ],
            tool_shapes: &["flat"],
            geometry: &["points"],
            machine_needs: &["xyz"],
            shapes_part: false,
        },
        OperationType {
            kind: "contour",
            title: "2D contour (3-axis)",
            description: "Follow a closed outline outside, inside or on it, in step-down passes.",
            group: "finishing",
            params: vec![
                tool(),
                p("index_a", "number", "deg", "Rotary setups: the A angle the part is turned to first (indexed 3+1); 0 on a vise.").opt(json!(0.0)),
                p("path", "points", "mm", "Closed outline as [[x, y], ...] (3 or more)."),
                p("side", "choice", "", "Which side of the outline the tool runs on.").choices(&["outside", "inside", "on"]),
                p("depth", "number", "mm", "Depth below the top.").min(0.01),
                p("top", "number", "mm", "Top (default: stock top).").opt(Value::Null),
                p("step_down", "number", "mm", "Depth per pass.").min(0.01),
                feed(),
                plunge(),
            ],
            tool_shapes: &["flat"],
            geometry: &["curve"],
            machine_needs: &["xyz"],
            shapes_part: false,
        },
        OperationType {
            kind: "raster3d",
            title: "3D raster (3-axis)",
            description: "Drop-cutter raster over the part mesh, passes along X stepping in Y, in step-down levels.",
            group: "finishing",
            params: vec![
                tool(),
                p("index_a", "number", "deg", "Rotary setups: the A angle the part is turned to first (indexed 3+1); 0 on a vise.").opt(json!(0.0)),
                p("x_range", "vec2", "mm", "X extent of the tool centre."),
                p("y_range", "vec2", "mm", "Y extent of the tool centre."),
                p("stepover", "number", "mm", "Distance between passes.").min(0.01),
                p("sample", "number", "mm", "Point spacing along a pass.").min(0.005),
                p("step_down", "number", "mm", "Depth per level (roughing to the surface).").min(0.01),
                p("floor", "number", "mm", "Lowest Z (where there is no part).").min(-10000.0),
                p("leave", "number", "mm", "Stock to leave on the surface.").opt(json!(0.0)),
                p("tolerance", "number", "mm", "Largest dip of a straight move below the surface.").opt(json!(0.005)).min(0.0001),
                feed(),
                plunge(),
            ],
            tool_shapes: &["flat", "ball", "vbit"],
            geometry: &["mesh"],
            machine_needs: &["xyz"],
            shapes_part: false,
        },
    ]
}

pub fn find(kind: &str) -> Option<OperationType> {
    registry().into_iter().find(|t| t.kind == kind)
}

fn tool_kind(s: &ToolShape) -> &'static str {
    match s {
        ToolShape::Flat { .. } => "flat",
        ToolShape::Ball { .. } => "ball",
        ToolShape::Vbit { .. } => "vbit",
    }
}

/// Check an operation's JSON against its type: unknown or missing parameters, types, ranges,
/// and (if `tools` is given) the tool's shape. Returns readable problems (empty = fine).
pub fn check(op: &Value, tools: Option<&[crate::job::Tool]>) -> Vec<String> {
    let mut out = Vec::new();
    let Some(kind) = op.get("type").and_then(Value::as_str) else {
        return vec!["operation has no \"type\"".into()];
    };
    let Some(t) = find(kind) else {
        return vec![format!("unknown operation type {kind:?}")];
    };
    let obj = op.as_object().cloned().unwrap_or_default();
    const COMMON: [&str; 5] = ["type", "id", "name", "color", "enabled"];
    for k in obj.keys() {
        if !COMMON.contains(&k.as_str()) && !t.params.iter().any(|p| p.name == k) {
            out.push(format!("{kind}: unknown parameter {k:?}"));
        }
    }
    for p in &t.params {
        let v = obj.get(p.name);
        let Some(v) = v.filter(|v| !v.is_null()) else {
            if p.required {
                out.push(format!("{kind}: missing {} ({})", p.name, p.help));
            }
            continue;
        };
        let range = |x: f64, out: &mut Vec<String>| {
            if p.min.is_some_and(|m| x < m) || p.max.is_some_and(|m| x > m) {
                out.push(format!(
                    "{kind}: {} = {x} is outside {}..{}",
                    p.name,
                    p.min.map_or("".into(), |m| m.to_string()),
                    p.max.map_or("".into(), |m| m.to_string())
                ));
            }
        };
        let ok = match p.kind {
            "number" => v.as_f64().map(|x| range(x, &mut out)).is_some(),
            "integer" => v.as_i64().map(|x| range(x as f64, &mut out)).is_some(),
            "boolean" => v.is_boolean(),
            "vec2" => v
                .as_array()
                .is_some_and(|a| a.len() == 2 && a.iter().all(Value::is_number)),
            "rect" => v.as_array().is_some_and(|a| {
                let n: Vec<f64> = a.iter().filter_map(Value::as_f64).collect();
                n.len() == 4 && a.len() == 4 && n[2] > n[0] && n[3] > n[1]
            }),
            "points" => v.as_array().is_some_and(|a| {
                let pairs = a.iter().all(|q| {
                    q.as_array()
                        .is_some_and(|q| q.len() == 2 && q.iter().all(Value::is_number))
                });
                pairs && a.len() >= if p.name == "path" { 3 } else { 1 }
            }),
            "numbers" => v
                .as_array()
                .is_some_and(|a| !a.is_empty() && a.iter().all(Value::is_number)),
            "integers" => v
                .as_array()
                .is_some_and(|a| !a.is_empty() && a.iter().all(|x| x.is_i64())),
            "choice" => v.as_str().is_some_and(|s| p.choices.contains(&s)),
            _ => true,
        };
        if !ok {
            out.push(format!(
                "{kind}: {} should be {}",
                p.name,
                match p.kind {
                    "vec2" => "[a, b]".to_string(),
                    "rect" => "[x0, y0, x1, y1] with x1 > x0 and y1 > y0".to_string(),
                    "points" if p.name == "path" =>
                        "[[x, y], ...] with 3 or more points".to_string(),
                    "points" => "[[x, y], ...]".to_string(),
                    "numbers" | "integers" => format!("a list of {}", p.kind),
                    "choice" => format!("one of {:?}", p.choices),
                    k => format!("a {k}"),
                }
            ));
        }
    }
    if let (Some(tools), Some(n)) = (tools, obj.get("tool").and_then(Value::as_u64)) {
        match tools.iter().find(|x| x.number as u64 == n) {
            None => out.push(format!("{kind}: tool T{n} is not defined")),
            Some(tl) if !t.tool_shapes.contains(&tool_kind(&tl.shape)) => out.push(format!(
                "{kind}: T{n} is a {} tool; this operation takes {}",
                tool_kind(&tl.shape),
                t.tool_shapes.join(" or ")
            )),
            _ => {}
        }
    }
    out
}

/// The `type` name of a parsed operation.
pub fn kind_of(op: &Operation) -> &'static str {
    match op {
        Operation::RotaryTurning(_) => "rotary_turning",
        Operation::HelicalKnurl(_) => "helical_knurl",
        Operation::FlankChamfer(_) => "flank_chamfer",
        Operation::RotaryFinish(_) => "rotary_finish",
        Operation::RotaryRough(_) => "rotary_rough",
        Operation::Face(_) => "face",
        Operation::Pocket(_) => "pocket",
        Operation::Drill(_) => "drill",
        Operation::Contour(_) => "contour",
        Operation::Raster3d(_) => "raster3d",
    }
}

// ------------------------------------------------------------------ defaults

/// What defaults are derived from.
pub struct DefaultsContext<'a> {
    /// The setup, lowered to a job (stock, tools, spindle, part).
    pub job: &'a crate::job::Job,
    /// The part's face regions (from the profile or mesh), if known.
    pub regions: Option<&'a [crate::geometry::FaceRegion]>,
    /// Machine limits (feed cap, spindle range).
    pub machine: Option<&'a crate::profiles::MachineProfile>,
    /// Chosen tool number (None: the first tool whose shape the operation accepts).
    pub tool: Option<u32>,
    /// Containment boundary from the operation's geometry selection.
    pub boundary: Option<crate::geometry::Boundary>,
    /// XY extent of the part mesh (3-axis setups), for raster ranges.
    pub part_xy: Option<([f64; 2], [f64; 2])>,
}

/// Round to a "nice" step (0.05 mm, 5 mm/min ...).
fn nice(v: f64, step: f64) -> f64 {
    ((v / step).round() * step * 1e6).round() / 1e6
}

/// A complete, valid operation of `kind` with every parameter filled in from the context:
/// tool size (depths, stepovers), spindle speed and the machine's feed cap (feeds), and the
/// part / selected geometry (ranges, knurl body, chamfer edge). Every host gets the same
/// defaults from here.
pub fn defaults(kind: &str, cx: &DefaultsContext) -> Result<Value, String> {
    let t = find(kind).ok_or(format!("unknown operation type {kind:?}"))?;
    let job = cx.job;
    // tool: the one asked for, else the first the operation accepts
    let tool = match cx.tool {
        Some(n) => job.tool(n).ok_or(format!("tool T{n} is not defined"))?,
        None => job
            .tools
            .iter()
            .find(|x| t.tool_shapes.contains(&tool_kind(&x.shape)))
            .ok_or(format!(
                "{kind} needs a {} tool; the job has none",
                t.tool_shapes.join(" or ")
            ))?,
    };
    let shape = tool_kind(&tool.shape);
    let d = match tool.shape {
        ToolShape::Flat { diameter } | ToolShape::Ball { diameter } => diameter,
        ToolShape::Vbit { diameter, .. } => diameter.unwrap_or(3.175),
    };
    // feeds: 0.025 mm per tooth (single flute) at the job's rpm, capped by the machine
    let cap = cx.machine.and_then(|m| m.max_feed).unwrap_or(f64::INFINITY);
    let rpm = job.spindle_rpm as f64;
    let feed = nice((0.025 * rpm).clamp(50.0, 0.75 * cap), 5.0);
    let plunge = nice((feed / 5.0).max(10.0), 5.0);
    // where: the selection's X range, else the part's
    let part_x = || -> Option<[f64; 2]> {
        if let Some(r) = cx.regions {
            let lo = r.iter().map(|f| f.x[0]).fold(f64::INFINITY, f64::min);
            let hi = r.iter().map(|f| f.x[1]).fold(f64::NEG_INFINITY, f64::max);
            if lo.is_finite() {
                return Some([lo, hi]);
            }
        }
        let p = job.part.profile.as_ref()?;
        Some([p.points.first()?[0], p.points.last()?[0]])
    };
    let x_range = cx
        .boundary
        .and_then(|b| b.x)
        .or_else(part_x)
        .unwrap_or([-10.0, 0.0]);
    let x_range = [nice(x_range[0], 0.001), nice(x_range[1], 0.001)];
    let a_range = cx.boundary.and_then(|b| b.a).unwrap_or([0.0, 360.0]);
    let mut v = serde_json::Map::new();
    v.insert("type".into(), json!(kind));
    v.insert("tool".into(), json!(tool.number));
    let depth = nice((0.125 * d).max(0.05), 0.05);
    let step = nice((0.3 * d).max(0.1), 0.05);
    let fine = nice((0.125 * d).max(0.05), 0.05);
    match kind {
        "rotary_turning" => {
            v.insert("rough_depth".into(), json!(depth));
            v.insert("rough_step".into(), json!(step));
            v.insert("rough_feed".into(), json!(feed));
            v.insert("finish_allowance".into(), json!(0.15));
            v.insert("finish_step".into(), json!(fine));
            v.insert("finish_feed".into(), json!(nice(0.85 * feed, 5.0)));
            v.insert("plunge_feed".into(), json!(plunge));
        }
        "rotary_rough" => {
            v.insert("x_range".into(), json!(x_range));
            v.insert("step_down".into(), json!(depth));
            v.insert("step_x".into(), json!(step));
            v.insert("allowance".into(), json!(0.15));
            v.insert("feed".into(), json!(feed));
            v.insert("plunge_feed".into(), json!(plunge));
        }
        "rotary_finish" => {
            let along = shape != "flat";
            v.insert(
                "pattern".into(),
                json!(if along { "along" } else { "around" }),
            );
            v.insert(
                "x_range".into(),
                json!(if along {
                    x_range
                } else {
                    [x_range[1], x_range[0]]
                }),
            );
            v.insert("a_range".into(), json!(a_range));
            if along {
                v.insert("stepover_deg".into(), json!(3.0));
            } else {
                v.insert("stepover_mm".into(), json!(fine));
            }
            v.insert("feed".into(), json!(nice(1.25 * feed, 5.0).min(0.75 * cap)));
            v.insert("plunge_feed".into(), json!(plunge));
        }
        "helical_knurl" => {
            let regions = cx
                .regions
                .ok_or("helical_knurl defaults need the part's faces")?;
            let body = regions
                .iter()
                .filter(|r| r.shape == "cone" || r.shape == "cylinder")
                .max_by(|a, b| a.area.total_cmp(&b.area))
                .ok_or("the part has no conical or cylindrical body to knurl")?;
            let (r0, r1) = crate::wizards::radius_at_ends(body, regions);
            let front = regions
                .iter()
                .map(|r| r.x[1])
                .fold(f64::NEG_INFINITY, f64::max);
            let (x0, x1) = (body.x[0], body.x[1]);
            let r_at = |x: f64| r0 + (r1 - r0) * (x - x0) / (x1 - x0);
            let r_mean = 0.5 * (r0 + r_at(front));
            // about 2 mm between grooves around the mean circumference
            let count = ((std::f64::consts::TAU * r_mean / 2.0).round() as u32).max(4);
            v.insert("body_back".into(), json!([nice(x0, 1e-6), nice(r0, 1e-6)]));
            v.insert(
                "body_front".into(),
                json!([nice(front, 1e-6), nice(r_at(front), 1e-6)]),
            );
            v.insert("count".into(), json!(count));
            v.insert("helix_angle".into(), json!(30.0));
            v.insert("fill".into(), json!(0.95));
            v.insert("passes".into(), json!([0.6, 1.0]));
            v.insert("feed".into(), json!(nice(feed * 2.0 / 3.0, 5.0)));
            v.insert("plunge_feed".into(), json!(plunge));
        }
        "flank_chamfer" => {
            let regions = cx
                .regions
                .ok_or("flank_chamfer defaults need the part's faces")?;
            let front = regions
                .iter()
                .map(|r| r.x[1])
                .fold(f64::NEG_INFINITY, f64::max);
            let face = regions
                .iter()
                .filter(|r| r.shape == "plane" && (r.x[0] - front).abs() < 1e-9)
                .max_by(|a, b| a.area.total_cmp(&b.area))
                .ok_or("the part has no front face to chamfer")?;
            v.insert(
                "edge".into(),
                json!([nice(front, 1e-6), nice(face.r[1], 1e-6)]),
            );
            v.insert("tip_offset".into(), json!(0.5));
            v.insert("passes".into(), json!([0.2, 0.0]));
            v.insert("feed".into(), json!(nice(feed * 2.0 / 3.0, 5.0)));
            v.insert("plunge_feed".into(), json!(plunge));
        }
        "face" | "pocket" | "drill" | "contour" | "raster3d" => {
            // the vise block, or around the bar over the part (indexed 3+1 on a rotary setup)
            let rad = job.stock.radius;
            let sb = job.stock_box.unwrap_or(crate::job::StockBox {
                min: [x_range[0], -rad, -rad],
                max: [x_range[1], rad, rad],
            });
            v.insert("index_a".into(), json!(0.0));
            let (w, h, hz) = (
                sb.max[0] - sb.min[0],
                sb.max[1] - sb.min[1],
                sb.max[2] - sb.min[2],
            );
            let (mx, my) = ((sb.min[0] + sb.max[0]) / 2.0, (sb.min[1] + sb.max[1]) / 2.0);
            if matches!(kind, "pocket" | "drill" | "contour") {
                v.insert("top".into(), json!(sb.max[2]));
            }
            match kind {
                "face" => {
                    v.insert("depth".into(), json!(0.5f64.min(hz)));
                    v.insert("step_down".into(), json!(depth.max(0.25)));
                    v.insert("stepover".into(), json!(0.6));
                }
                "pocket" => {
                    v.insert(
                        "rect".into(),
                        json!([
                            nice(mx - w / 4.0, 0.01),
                            nice(my - h / 4.0, 0.01),
                            nice(mx + w / 4.0, 0.01),
                            nice(my + h / 4.0, 0.01)
                        ]),
                    );
                    v.insert("depth".into(), json!(nice((0.3 * hz).max(0.5), 0.05)));
                    v.insert("step_down".into(), json!(depth));
                    v.insert("stepover".into(), json!(0.5));
                }
                "drill" => {
                    v.insert("points".into(), json!([[nice(mx, 0.01), nice(my, 0.01)]]));
                    v.insert("depth".into(), json!(nice((0.5 * hz).max(0.5), 0.05)));
                    v.insert("peck".into(), json!(nice(d.max(0.5), 0.05)));
                    v.insert("retract".into(), json!(1.0));
                    v.insert("feed".into(), json!(plunge));
                }
                "contour" => {
                    let i = 2.0f64.min(w / 4.0).min(h / 4.0);
                    let (x0, y0, x1, y1) =
                        (sb.min[0] + i, sb.min[1] + i, sb.max[0] - i, sb.max[1] - i);
                    v.insert(
                        "path".into(),
                        json!([[x0, y0], [x1, y0], [x1, y1], [x0, y1]]),
                    );
                    v.insert("side".into(), json!("outside"));
                    v.insert("depth".into(), json!(nice((0.3 * hz).max(0.5), 0.05)));
                    v.insert("step_down".into(), json!(depth));
                }
                _ => {
                    // over the part mesh, or the block
                    let (xr, yr) = cx
                        .part_xy
                        .unwrap_or(([sb.min[0], sb.max[0]], [sb.min[1], sb.max[1]]));
                    let zb = sb.min[2];
                    v.insert(
                        "x_range".into(),
                        json!([nice(xr[0], 0.01), nice(xr[1], 0.01)]),
                    );
                    v.insert(
                        "y_range".into(),
                        json!([nice(yr[0], 0.01), nice(yr[1], 0.01)]),
                    );
                    v.insert("stepover".into(), json!(nice((0.15 * d).max(0.05), 0.01)));
                    v.insert("sample".into(), json!(nice((0.1 * d).max(0.05), 0.01)));
                    v.insert("step_down".into(), json!(depth));
                    v.insert(
                        "floor".into(),
                        json!(nice(zb.max(sb.max[2] - hz / 2.0), 0.01)),
                    );
                }
            }
            if !v.contains_key("feed") {
                v.insert("feed".into(), json!(feed));
            }
            if kind != "drill" {
                v.insert("plunge_feed".into(), json!(plunge));
            }
        }
        _ => return Err(format!("no defaults for {kind:?}")),
    }
    // every other parameter: its registry default
    for p in &t.params {
        if !v.contains_key(p.name) && !p.default.is_null() {
            v.insert(p.name.into(), p.default.clone());
        }
    }
    let out = Value::Object(v);
    let problems = check(&out, Some(&job.tools));
    if !problems.is_empty() {
        return Err(format!(
            "defaults for {kind} are not valid: {}",
            problems.join("; ")
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::Job;

    fn sample(t: &OperationType) -> Value {
        let mut m = serde_json::Map::new();
        m.insert("type".into(), json!(t.kind));
        for p in &t.params {
            let v = if !p.default.is_null() {
                p.default.clone()
            } else {
                match p.kind {
                    "integer" => json!(p.min.unwrap_or(1.0).max(1.0) as i64),
                    "vec2" => json!([-5.0, 0.0]),
                    "rect" => json!([5.0, 5.0, 25.0, 35.0]),
                    "points" => json!([[2.0, 2.0], [58.0, 2.0], [58.0, 38.0], [2.0, 38.0]]),
                    "numbers" => json!([1.0]),
                    "integers" => json!([1]),
                    "choice" => json!(p.choices[0]),
                    "boolean" => json!(true),
                    _ => json!(p.min.unwrap_or(1.0).max(0.5)),
                }
            };
            m.insert(p.name.into(), v);
        }
        if t.kind == "rotary_finish" {
            m.insert("stepover_mm".into(), json!(0.5));
        }
        Value::Object(m)
    }

    #[test]
    fn every_operation_type_is_registered_and_its_spec_matches_the_engine() {
        let reg = registry();
        let kinds: Vec<&str> = reg.iter().map(|t| t.kind).collect();
        let job = Job::from_json(include_str!("../../../examples/knob/knob.job.json")).unwrap();
        for prog in &job.programs {
            for op in &prog.operations {
                assert!(kinds.contains(&kind_of(op)));
            }
        }
        for t in &reg {
            // a sample built from the spec parses as that operation ...
            let s = sample(t);
            let op: Operation =
                serde_json::from_value(s.clone()).unwrap_or_else(|e| panic!("{}: {e}", t.kind));
            assert_eq!(kind_of(&op), t.kind);
            assert!(
                check(&s, None).is_empty(),
                "{}: {:?}",
                t.kind,
                check(&s, None)
            );
            // ... and every field the engine writes is in the spec
            let back = serde_json::to_value(&op).unwrap();
            for k in back.as_object().unwrap().keys() {
                assert!(
                    k == "type" || t.params.iter().any(|p| p.name == k),
                    "{}: {k} missing from spec",
                    t.kind
                );
            }
        }
    }

    #[test]
    fn defaults_for_every_type_are_complete_valid_and_plan_on_the_knob() {
        let job = Job::from_json(include_str!("../../../examples/knob/knob.job.json")).unwrap();
        let mesh = crate::Mesh::revolve(job.part.profile.as_ref().unwrap(), 360);
        let (regions, _) = crate::geometry::face_regions(&mesh, 30.0);
        let z1 = crate::profiles::builtin_machine("makera-z1-4axis").unwrap();
        // tools on hand for every shape: the knob's flat T1 and V-bit T2, plus a ball T3
        let mut job = job;
        job.tools.push(crate::job::Tool {
            number: 3,
            description: "ball 3.175".into(),
            shape: ToolShape::Ball { diameter: 3.175 },
            flute_length: Some(12.0),
        });
        let mut boxed = job.clone();
        boxed.stock_box = Some(crate::job::StockBox {
            min: [0.0, 0.0, -10.0],
            max: [60.0, 40.0, 0.0],
        });
        let dome =
            crate::Mesh::from_bytes(include_bytes!("../../../examples/plate/dome.stl")).unwrap();
        for t in registry() {
            let three = t.machine_needs.contains(&"xyz");
            let job = if three { &boxed } else { &job };
            let cx = DefaultsContext {
                job,
                regions: Some(&regions),
                machine: Some(&z1),
                tool: None,
                boundary: None,
                part_xy: None,
            };
            let v = defaults(t.kind, &cx).unwrap_or_else(|e| panic!("{}: {e}", t.kind));
            // every parameter present (none blank)
            for p in &t.params {
                let has = v.get(p.name).is_some_and(|x| !x.is_null());
                let either = matches!(p.name, "stepover_deg" | "stepover_mm"); // one of the two
                assert!(has || either, "{}: {} has no default", t.kind, p.name);
            }
            assert!(
                check(&v, Some(&job.tools)).is_empty(),
                "{}: {:?}",
                t.kind,
                check(&v, Some(&job.tools))
            );
            // feeds respect the machine's cap
            for k in ["feed", "rough_feed", "finish_feed"] {
                if let Some(f) = v.get(k).and_then(Value::as_f64) {
                    assert!(f <= 0.75 * 4000.0, "{}: {k} {f}", t.kind);
                }
            }
            // and it plans on the knob setup
            let op: Operation = serde_json::from_value(v.clone()).unwrap();
            let mut one = job.clone();
            one.programs = vec![crate::job::Program {
                name: "t".into(),
                title: "t".into(),
                notes: vec![],
                reset_a: false,
                operations: vec![op],
            }];
            let tps = crate::plan_with_mesh(&one, three.then_some(&dome))
                .unwrap_or_else(|e| panic!("{}: {e}", t.kind));
            assert!(tps[0].moves.len() > 10, "{}", t.kind);
        }
        // defaults follow the context: the knob's own values come out for the knob
        let cx = DefaultsContext {
            job: &job,
            regions: Some(&regions),
            machine: Some(&z1),
            tool: None,
            boundary: None,
            part_xy: None,
        };
        let turn = defaults("rotary_turning", &cx).unwrap();
        assert_eq!(
            (
                turn["rough_depth"].as_f64(),
                turn["rough_step"].as_f64(),
                turn["rough_feed"].as_f64(),
                turn["plunge_feed"].as_f64()
            ),
            (Some(0.4), Some(0.95), Some(300.0), Some(60.0))
        );
        let knurl = defaults("helical_knurl", &cx).unwrap();
        assert_eq!(knurl["body_back"], json!([-15.0, 10.0]));
        assert_eq!(knurl["body_front"], json!([0.0, 8.0]));
        assert_eq!(knurl["count"], json!(28));
        let chamfer = defaults("flank_chamfer", &cx).unwrap();
        assert_eq!(chamfer["edge"], json!([0.0, 7.2]));
        // a ball tool finishes along X, a flat one around; a selection sets the range
        let b = crate::geometry::Boundary {
            x: Some([-15.0, -1.0]),
            a: None,
        };
        let ball = defaults(
            "rotary_finish",
            &DefaultsContext {
                tool: Some(3),
                boundary: Some(b),
                ..cx
            },
        )
        .unwrap();
        assert_eq!(
            (ball["pattern"].as_str(), ball["x_range"].clone()),
            (Some("along"), json!([-15.0, -1.0]))
        );
        let flat = defaults(
            "rotary_finish",
            &DefaultsContext {
                tool: Some(1),
                ..cx
            },
        )
        .unwrap();
        assert_eq!(flat["pattern"], "around");
        // a slow machine caps the feeds
        let mut slow = z1.clone();
        slow.max_feed = Some(200.0);
        let capped = defaults(
            "rotary_turning",
            &DefaultsContext {
                machine: Some(&slow),
                ..cx
            },
        )
        .unwrap();
        assert!(capped["rough_feed"].as_f64().unwrap() <= 150.0);
        assert!(defaults(
            "helical_knurl",
            &DefaultsContext {
                tool: Some(1),
                ..cx
            }
        )
        .is_err());
    }

    #[test]
    fn check_reports_problems() {
        let job = Job::from_json(include_str!("../../../examples/knob/knob.job.json")).unwrap();
        let knurl = serde_json::to_value(&job.programs[1].operations[0]).unwrap();
        assert!(check(&knurl, Some(&job.tools)).is_empty());
        let mut bad = knurl.clone();
        bad["tool"] = json!(1);
        bad["fill"] = json!(1.5);
        bad["passes"] = json!("x");
        bad["bogus"] = json!(1);
        bad.as_object_mut().unwrap().remove("count");
        let errs = check(&bad, Some(&job.tools)).join("\n");
        for want in [
            "T1 is a flat tool",
            "fill = 1.5 is outside",
            "passes should be",
            "unknown parameter \"bogus\"",
            "missing count",
        ] {
            assert!(errs.contains(want), "{want}: {errs}");
        }
        assert!(check(&json!({"type": "nope"}), None)[0].contains("unknown operation type"));
    }
}
