//! A one-setup project around a part mesh, for CAD adapters (SolidCraft, FreeCAD, ...): the
//! body placed on the rotary axis, bar stock, a tool and one operation, every field filled
//! by marcam's defaults with the inputs given on top. The adapters then plan and post it like
//! any project, so they all behave the same.
//!
//! Inputs (all optional), besides the operation's own fields:
//!
//! | Input | Default |
//! | --- | --- |
//! | `body` | the body's name, for the program and file names (`"body"`) |
//! | `axis` | `"X"`: the mesh axis (through its origin) that becomes the rotary axis |
//! | `machine` | `makera-z1-4axis`, any built-in machine |
//! | `tool_kind` | the operation's first tool kind |
//! | `tool_diameter` | 3.175 mm |
//! | `vbit_angle` | 90° (V-bits) |
//! | `tip_diameter`, `taper_angle` | 1 mm, 3° a side (tapered balls) |
//! | `stock_radius` | the body's largest radius + 0.5 mm, rounded up to 0.5 mm |
//! | `stock_length` | the body's length + 15 mm, rounded up |
//! | `spindle_rpm` | 12000 |
//! | `name` | `<body>_<operation>`: names the program and its files |
//!
//! The body is turned so `axis` runs along X and moved so its front (largest X) is at X0.

use crate::det::Det;
use crate::job::MeshSource;
use crate::mesh::Axis;
use crate::{ops, profiles};
use serde_json::{json, Value};

/// The machine used when none is given.
pub const DEFAULT_MACHINE: &str = "makera-z1-4axis";
/// Tool diameter when none is given (mm).
pub const DEFAULT_TOOL_DIAMETER: f64 = 3.175;
/// V-bit included angle when none is given (deg).
pub const DEFAULT_VBIT_ANGLE: f64 = 90.0;
/// Tapered ball tip diameter when none is given (mm).
pub const DEFAULT_TIP_DIAMETER: f64 = 1.0;
/// Tapered ball side angle when none is given (deg).
pub const DEFAULT_TAPER_ANGLE: f64 = 3.0;
/// Spindle speed when none is given (rpm).
pub const DEFAULT_RPM: u32 = 12000;
/// Default stock: the body's largest radius plus this, rounded up to 0.5 mm (mm).
pub const STOCK_RADIAL_MARGIN: f64 = 0.5;
/// Default stock: the body's length plus this much to hold (mm).
pub const STOCK_GRIP: f64 = 15.0;

/// Inputs every operation takes here, besides its own fields.
pub const INPUTS: [&str; 12] = [
    "body",
    "axis",
    "machine",
    "tool_kind",
    "tool_diameter",
    "vbit_angle",
    "tip_diameter",
    "taper_angle",
    "stock_radius",
    "stock_length",
    "spindle_rpm",
    "name",
];

/// Why a project could not be made: an input (named) or the engine.
#[derive(Debug, Clone, PartialEq)]
pub enum BodyError {
    Input { field: String, reason: String },
    Engine(String),
}

impl std::fmt::Display for BodyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BodyError::Input { field, reason } => write!(f, "{field}: {reason}"),
            BodyError::Engine(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for BodyError {}

fn input(field: &str, reason: impl Into<String>) -> BodyError {
    BodyError::Input {
        field: field.into(),
        reason: reason.into(),
    }
}

fn opt_f64(params: &Value, field: &str) -> Result<Option<f64>, BodyError> {
    match params.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_f64()
            .filter(|x| x.is_finite() && *x > 0.0)
            .map(Some)
            .ok_or_else(|| input(field, "a positive number")),
    }
}

fn opt_str<'a>(params: &'a Value, field: &str) -> Result<Option<&'a str>, BodyError> {
    match params.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_str().map(Some).ok_or_else(|| input(field, "text")),
    }
}

fn round_up_half(x: f64) -> f64 {
    (x * 2.0).ceil() / 2.0
}

/// The project for operation `kind` on the body in `stl` (STL or 3MF bytes, the body's own
/// coordinates in mm): returns it and the program name. `params` holds the inputs (see the
/// module) and any of the operation's fields; `extra` names further keys an adapter uses
/// itself and that are allowed through. The project's mesh model names `<name>.stl`, so it
/// plans from the same bytes saved under that name.
pub fn body_project(
    kind: &str,
    params: &Value,
    stl: &[u8],
    extra: &[&str],
) -> Result<(Value, String), BodyError> {
    let t = ops::find(kind)
        .ok_or_else(|| input("operation", format!("no marcam operation {kind:?}")))?;
    if t.geometry == ["profile"] {
        return Err(input(
            "operation",
            format!("{kind} works from a revolved profile, not a mesh"),
        ));
    }
    if !params.is_object() {
        return Err(input("params", "an object"));
    }
    for (k, _) in params.as_object().into_iter().flatten() {
        let known = INPUTS.contains(&k.as_str())
            || extra.contains(&k.as_str())
            || t.params.iter().any(|p| p.name == k && k != "tool");
        if !known {
            return Err(input(k, format!("not an input of {kind}")));
        }
    }
    let axis = match opt_str(params, "axis")?.unwrap_or("X") {
        "X" | "x" => Axis::X,
        "Y" | "y" => Axis::Y,
        "Z" | "z" => Axis::Z,
        _ => return Err(input("axis", "X, Y or Z")),
    };
    let machine_id = opt_str(params, "machine")?.unwrap_or(DEFAULT_MACHINE);
    let machine = profiles::builtin_machine(machine_id)
        .ok_or_else(|| input("machine", format!("no built-in machine {machine_id:?}")))?;
    let body = match opt_str(params, "body")? {
        Some(b) => b.to_string(),
        // SolidCraft passes its selection as `bodies`
        None => params["bodies"][0].as_str().unwrap_or("body").to_string(),
    };
    let name = match opt_str(params, "name")? {
        Some(n) => n.to_string(),
        None => {
            let clean: String = body
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() {
                        c.to_ascii_lowercase()
                    } else {
                        '_'
                    }
                })
                .collect();
            format!("{}_{kind}", clean.trim_matches('_'))
        }
    };
    if name.is_empty()
        || name.len() > 64
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(input("name", "1..64 letters, digits, _ or -"));
    }

    // Place the body: its axis on X, its front (largest X) at X0.
    let axis_name = match axis {
        Axis::X => "x",
        Axis::Y => "y",
        Axis::Z => "z",
    };
    let file = format!("{name}.stl");
    let mut source = MeshSource {
        file: file.clone(),
        axis,
        turn: Default::default(),
        translate: [0.0; 3],
        image: None,
    };
    let raw = source
        .mesh(stl)
        .map_err(|e| BodyError::Engine(format!("body mesh: {e}")))?;
    let (lo, hi) = raw
        .bounds()
        .ok_or_else(|| BodyError::Engine("the body has no triangles".into()))?;
    source.translate = [-hi[0], 0.0, 0.0];
    let rmax = raw
        .triangles
        .iter()
        .flatten()
        .map(|p| p[1].dhypot(p[2]))
        .fold(0.0, f64::max);
    let stock_radius = match opt_f64(params, "stock_radius")? {
        Some(r) => r,
        // (less a hair, so f32 rounding of an exact radius does not add 0.5 mm)
        None => round_up_half(rmax + STOCK_RADIAL_MARGIN - 1e-4),
    };
    let stock_length = match opt_f64(params, "stock_length")? {
        Some(l) => l,
        None => (hi[0] - lo[0] + STOCK_GRIP).ceil(),
    };

    let tool_kind = opt_str(params, "tool_kind")?.unwrap_or(t.tool_shapes[0]);
    if !t.tool_shapes.contains(&tool_kind) {
        return Err(input(
            "tool_kind",
            format!("{kind} takes {}", t.tool_shapes.join(", ")),
        ));
    }
    let diameter = opt_f64(params, "tool_diameter")?.unwrap_or(DEFAULT_TOOL_DIAMETER);
    let tool = match tool_kind {
        "vbit" => {
            let angle = opt_f64(params, "vbit_angle")?.unwrap_or(DEFAULT_VBIT_ANGLE);
            json!({"number": 1, "description": format!("{angle} deg V-bit, {diameter} mm"),
                   "kind": "vbit", "angle": angle, "diameter": diameter})
        }
        "tapered_ball" => {
            let tip = opt_f64(params, "tip_diameter")?.unwrap_or(DEFAULT_TIP_DIAMETER);
            let taper = opt_f64(params, "taper_angle")?.unwrap_or(DEFAULT_TAPER_ANGLE);
            json!({"number": 1, "description": format!("tapered ball, {tip} mm tip, {taper} deg a side, {diameter} mm"),
                   "kind": "tapered_ball", "tip_diameter": tip, "taper_angle": taper, "diameter": diameter})
        }
        k => json!({"number": 1, "description": format!("{diameter} mm {k} end mill"),
                    "kind": k, "diameter": diameter}),
    };
    let rpm = match params.get("spindle_rpm") {
        None | Some(Value::Null) => DEFAULT_RPM as u64,
        Some(v) => v
            .as_u64()
            .filter(|r| *r > 0 && *r <= u32::MAX as u64)
            .ok_or_else(|| input("spindle_rpm", "a positive whole number"))?,
    };

    let mut project = json!({
        "schema_version": "0.2",
        "name": name,
        "models": [{
            "id": "body", "name": body, "kind": "mesh", "file": file,
            "axis": axis_name, "translate": source.translate,
        }],
        "tools": [tool],
        "setups": [{
            "id": "setup1",
            "name": "Rotary setup",
            "machine": machine.id,
            "stock": {"shape": "cylinder", "radius": stock_radius, "length": stock_length},
            "clearance": 3.0,
            "holding": {"kind": "rotary_chuck", "tailstock": false},
            "part": ["body"],
            "spindle_rpm": rpm,
            "operations": [],
            "outputs": [],
        }],
    });

    // The operation: marcam's defaults, then the fields given.
    let text = project.to_string();
    let (p, _) = crate::project::Project::from_json_any(&text).map_err(BodyError::Engine)?;
    let placed = source
        .mesh(stl)
        .map_err(|e| BodyError::Engine(e.to_string()))?;
    let meshes = |_: &str| Some(placed.clone());
    let mut op = p
        .op_defaults(kind, "setup1", Some(1), &[], &[], Some(&machine), &meshes)
        .map_err(BodyError::Engine)?;
    for spec in &t.params {
        if let Some(v) = params
            .get(spec.name)
            .filter(|v| !v.is_null() && spec.name != "tool")
        {
            op[spec.name] = v.clone();
        }
    }
    op["id"] = json!("op1");
    op["enabled"] = json!(true);
    let problems = ops::check(&op, None);
    if !problems.is_empty() {
        return Err(input(kind, problems.join("; ")));
    }
    let setup = &mut project["setups"][0];
    setup["operations"] = json!([op]);
    setup["outputs"] = json!([{
        "name": name,
        "title": format!("{} on {body}", t.title),
        "operations": ["op1"],
    }]);
    Ok((project, name))
}
