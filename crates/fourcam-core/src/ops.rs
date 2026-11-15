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
    /// "number", "integer", "boolean", "vec2" ([a, b]), "numbers" (list), "integers", "choice"
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
    }
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
