//! Document model 0.2: a Project holds models, tools and setups; each setup has a machine,
//! stock, holding and an ordered list of operations, grouped into output programs (G-code
//! files). 0.1 jobs upgrade automatically. Today a setup is planned by lowering it to a 0.1
//! [`Job`]; the Z1 knob programs come out byte-identical either way.
//!
//! Operation ids are strings unique within the project; they are what colours, simulation
//! tags and (later) the frontend's operation tree key off.

use crate::job::{self, Job, MeshSource, Operation, Profile, Stock, Tool};
use crate::profiles::{self, ToolLibrary};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROJECT_SCHEMA: &str = "0.2";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Project {
    pub schema_version: String,
    #[serde(default)]
    pub name: String,
    pub models: Vec<Model>,
    /// Tools by T number: inline definitions or library references.
    pub tools: Vec<ToolEntry>,
    pub setups: Vec<Setup>,
}

/// Part geometry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Model {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(flatten)]
    pub kind: ModelKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelKind {
    /// Revolved profile about the rotary axis.
    Profile(Profile),
    /// Triangle mesh file (STL/3MF), placed on the axis.
    Mesh(MeshSource),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ToolEntry {
    /// `{"number": 1, "library": "makera-starter", "tool": "..."}`
    Library {
        number: u32,
        library: String,
        tool: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    Inline(Tool),
}

impl ToolEntry {
    pub fn number(&self) -> u32 {
        match self {
            ToolEntry::Library { number, .. } => *number,
            ToolEntry::Inline(t) => t.number,
        }
    }
}

/// How the work is held.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Holding {
    /// "rotary_chuck" (today) or later "vise", "fixture".
    pub kind: String,
    #[serde(default)]
    pub tailstock: bool,
}

impl Default for Holding {
    fn default() -> Self {
        Holding {
            kind: "rotary_chuck".into(),
            tailstock: false,
        }
    }
}

/// Stock shape. A setup with `from_previous` starts from the previous setup's result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum StockDef {
    Cylinder {
        radius: f64,
        length: f64,
    },
    /// The stock left by the previous setup (simulated); its outer radius is used for planning.
    FromPrevious {
        radius: f64,
        length: f64,
    },
}

impl StockDef {
    pub fn cylinder(&self) -> Stock {
        match *self {
            StockDef::Cylinder { radius, length } | StockDef::FromPrevious { radius, length } => {
                Stock { radius, length }
            }
        }
    }
}

/// An operation in a setup.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectOp {
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Display colour for the material it removes (`#rrggbb`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Where it may cut (empty = the whole model). See [`crate::geometry::Selection`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub geometry: Vec<crate::geometry::Selection>,
    #[serde(flatten)]
    pub op: Operation,
}

fn yes() -> bool {
    true
}
fn default_clearance() -> f64 {
    3.0
}

/// One G-code file: operations in order, plus header notes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Output {
    pub name: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(default)]
    pub reset_a: bool,
    /// Operation ids.
    pub operations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Setup {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    pub stock: StockDef,
    #[serde(default = "default_clearance")]
    pub clearance: f64,
    #[serde(default)]
    pub holding: Holding,
    /// Model ids that make up the finished part (a profile and/or a mesh).
    pub part: Vec<String>,
    pub spindle_rpm: u32,
    pub operations: Vec<ProjectOp>,
    pub outputs: Vec<Output>,
}

impl Project {
    /// Read a 0.2 project, or a 0.1 job (upgraded). Returns whether it was upgraded.
    pub fn from_json_any(text: &str) -> Result<(Project, bool), String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
        match v.get("schema_version").and_then(Value::as_str) {
            Some(job::SCHEMA_VERSION) => {
                let job: Job = serde_json::from_value(v).map_err(|e| format!("job 0.1: {e}"))?;
                Ok((Project::from_job(&job, "knob"), true))
            }
            Some(PROJECT_SCHEMA) => {
                let p: Project =
                    serde_json::from_value(v).map_err(|e| format!("project 0.2: {e}"))?;
                p.validate()?;
                Ok((p, false))
            }
            other => Err(format!(
                "unknown schema_version {other:?} (expected 0.1 or 0.2)"
            )),
        }
    }

    /// Upgrade a 0.1 job: one setup holding all its programs as outputs.
    pub fn from_job(job: &Job, name: &str) -> Project {
        let mut models = Vec::new();
        let mut part = Vec::new();
        if let Some(p) = &job.part.profile {
            models.push(Model {
                id: "profile".into(),
                name: "revolved profile".into(),
                kind: ModelKind::Profile(p.clone()),
            });
            part.push("profile".into());
        }
        if let Some(m) = &job.part.mesh {
            models.push(Model {
                id: "mesh".into(),
                name: m.file.clone(),
                kind: ModelKind::Mesh(m.clone()),
            });
            part.push("mesh".into());
        }
        let mut operations = Vec::new();
        let mut outputs = Vec::new();
        for prog in &job.programs {
            let mut ids = Vec::new();
            for op in &prog.operations {
                let id = format!("op{}", operations.len() + 1);
                ids.push(id.clone());
                operations.push(ProjectOp {
                    id,
                    name: String::new(),
                    color: None,
                    enabled: true,
                    geometry: Vec::new(),
                    op: op.clone(),
                });
            }
            outputs.push(Output {
                name: prog.name.clone(),
                title: prog.title.clone(),
                notes: prog.notes.clone(),
                reset_a: prog.reset_a,
                operations: ids,
            });
        }
        Project {
            schema_version: PROJECT_SCHEMA.into(),
            name: name.into(),
            models,
            tools: job.tools.iter().cloned().map(ToolEntry::Inline).collect(),
            setups: vec![Setup {
                id: "setup1".into(),
                name: "Rotary setup".into(),
                machine: job.machine.clone(),
                stock: StockDef::Cylinder {
                    radius: job.stock.radius,
                    length: job.stock.length,
                },
                clearance: job.clearance,
                holding: Holding::default(),
                part,
                spindle_rpm: job.spindle_rpm,
                operations,
                outputs,
            }],
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != PROJECT_SCHEMA {
            return Err(format!("project schema_version must be {PROJECT_SCHEMA:?}"));
        }
        let mut ids = std::collections::HashSet::new();
        for m in &self.models {
            if !ids.insert(format!("model:{}", m.id)) {
                return Err(format!("model id {:?} is used twice", m.id));
            }
        }
        let mut numbers = std::collections::HashSet::new();
        for t in &self.tools {
            if !numbers.insert(t.number()) {
                return Err(format!("tool number T{} is used twice", t.number()));
            }
        }
        let mut setups = std::collections::HashSet::new();
        let mut outputs = std::collections::HashSet::new();
        for (k, s) in self.setups.iter().enumerate() {
            if k == 0 && matches!(s.stock, StockDef::FromPrevious { .. }) {
                return Err(format!(
                    "setup {:?}: the first setup cannot take its stock from a previous one",
                    s.id
                ));
            }
            for out in &s.outputs {
                if !outputs.insert(out.name.clone()) {
                    return Err(format!(
                        "output name {:?} is used twice (G-code files would overwrite each other)",
                        out.name
                    ));
                }
            }
            if !setups.insert(&s.id) {
                return Err(format!("setup id {:?} is used twice", s.id));
            }
            for p in &s.part {
                if !self.models.iter().any(|m| &m.id == p) {
                    return Err(format!("setup {:?}: unknown model {p:?}", s.id));
                }
            }
            let mut ops = std::collections::HashSet::new();
            for o in &s.operations {
                if !ids.insert(format!("op:{}", o.id)) || !ops.insert(o.id.as_str()) {
                    return Err(format!("operation id {:?} is used twice", o.id));
                }
                if !numbers.contains(&o.op.tool()) {
                    return Err(format!(
                        "operation {:?}: tool T{} is not defined",
                        o.id,
                        o.op.tool()
                    ));
                }
            }
            for out in &s.outputs {
                for id in &out.operations {
                    if !ops.contains(id.as_str()) {
                        return Err(format!("output {:?}: unknown operation {id:?}", out.name));
                    }
                }
            }
        }
        Ok(())
    }

    /// Check every operation against the operation registry (parameters, ranges, tool shapes
    /// for inline tools). Problems are prefixed with the setup and operation id.
    pub fn check_operations(&self) -> Vec<String> {
        let inline: Vec<Tool> = self
            .tools
            .iter()
            .filter_map(|t| match t {
                ToolEntry::Inline(t) => Some(t.clone()),
                _ => None,
            })
            .collect();
        let mut out = Vec::new();
        for s in &self.setups {
            for o in &s.operations {
                let v = serde_json::to_value(&o.op).unwrap_or(Value::Null);
                let has_inline = inline.iter().any(|t| t.number == o.op.tool());
                for e in crate::ops::check(&v, has_inline.then_some(inline.as_slice())) {
                    out.push(format!("{} / {}: {e}", s.id, o.id));
                }
            }
        }
        out
    }

    /// Defaults for a new operation in a setup: every parameter filled from the setup's
    /// tools, spindle speed, machine limits, part faces and the given geometry selection.
    #[allow(clippy::too_many_arguments)]
    pub fn op_defaults(
        &self,
        kind: &str,
        setup: &str,
        tool: Option<u32>,
        geometry: &[crate::geometry::Selection],
        libraries: &[ToolLibrary],
        machine: Option<&crate::profiles::MachineProfile>,
        meshes: &dyn Fn(&str) -> Option<crate::Mesh>,
    ) -> Result<Value, String> {
        let s = self.setup(setup).ok_or(format!("no setup {setup:?}"))?;
        let job = self.setup_job_raw(s, libraries)?;
        // faces of the part: the mesh model if any, else the profile
        let model = s
            .part
            .iter()
            .find(|id| {
                self.models
                    .iter()
                    .any(|m| &m.id == *id && matches!(m.kind, ModelKind::Mesh(_)))
            })
            .or(s.part.first());
        let regions = model.and_then(|m| self.model_regions(m, meshes));
        let radius = tool
            .and_then(|n| job.tool(n))
            .map_or(0.0, |t| match t.shape {
                job::ToolShape::Flat { diameter } | job::ToolShape::Ball { diameter } => {
                    diameter / 2.0
                }
                job::ToolShape::Vbit { diameter, .. } => diameter.unwrap_or(0.0) / 2.0,
            });
        let boundary = if geometry.is_empty() {
            None
        } else {
            Some(crate::geometry::boundary(
                geometry,
                &|m| self.model_regions(m, meshes),
                radius,
            )?)
        };
        let machine_owned;
        let machine = match machine {
            Some(m) => Some(m),
            None => {
                machine_owned = s.machine.as_deref().and_then(profiles::builtin_machine);
                machine_owned.as_ref()
            }
        };
        let mut v = crate::ops::defaults(
            kind,
            &crate::ops::DefaultsContext {
                job: &job,
                regions: regions.as_deref(),
                machine,
                tool,
                boundary,
            },
        )?;
        if !geometry.is_empty() {
            v["geometry"] = serde_json::to_value(geometry).map_err(|e| e.to_string())?;
        }
        Ok(v)
    }

    pub fn setup(&self, id: &str) -> Option<&Setup> {
        self.setups.iter().find(|s| s.id == id)
    }

    /// The 0.1 job that plans a setup (enabled operations only; empty outputs dropped).
    /// Face selections on mesh models need [`Project::setup_job_with`].
    pub fn setup_job(&self, setup: &Setup, libraries: &[ToolLibrary]) -> Result<Job, String> {
        self.setup_job_with(setup, libraries, &|_| None)
    }

    /// Face regions of a model: profiles are revolved (360 facets); meshes come from `meshes`.
    pub fn model_regions(
        &self,
        model: &str,
        meshes: &dyn Fn(&str) -> Option<crate::Mesh>,
    ) -> Option<Vec<crate::geometry::FaceRegion>> {
        let m = self.models.iter().find(|m| m.id == model)?;
        let mesh = match &m.kind {
            ModelKind::Profile(p) => crate::Mesh::revolve(p, 360),
            ModelKind::Mesh(_) => meshes(model)?,
        };
        Some(crate::geometry::face_regions(&mesh, 30.0).0)
    }

    /// Like [`Project::setup_job`], with mesh models available for face selections.
    pub fn setup_job_with(
        &self,
        setup: &Setup,
        libraries: &[ToolLibrary],
        meshes: &dyn Fn(&str) -> Option<crate::Mesh>,
    ) -> Result<Job, String> {
        let mut job = self.setup_job_raw(setup, libraries)?;
        // apply geometry selections as containment boundaries
        for o in setup
            .operations
            .iter()
            .filter(|o| o.enabled && !o.geometry.is_empty())
        {
            let radius = job
                .tool(o.op.tool())
                .map(|t| match t.shape {
                    job::ToolShape::Flat { diameter } | job::ToolShape::Ball { diameter } => {
                        diameter / 2.0
                    }
                    job::ToolShape::Vbit { diameter, .. } => diameter.unwrap_or(0.0) / 2.0,
                })
                .unwrap_or(0.0);
            let b =
                crate::geometry::boundary(&o.geometry, &|m| self.model_regions(m, meshes), radius)
                    .map_err(|e| format!("operation {:?}: {e}", o.id))?;
            let target = job
                .programs
                .iter_mut()
                .flat_map(|p| p.operations.iter_mut())
                .find(|op| **op == o.op)
                .ok_or(format!("operation {:?} not found in the plan", o.id))?;
            let clamp = |r: [f64; 2], b: [f64; 2]| -> [f64; 2] {
                let (lo, hi) = (r[0].min(r[1]).max(b[0]), r[0].max(r[1]).min(b[1]));
                if r[0] <= r[1] {
                    [lo, hi]
                } else {
                    [hi, lo]
                }
            };
            match target {
                Operation::RotaryFinish(f) => {
                    if let Some(x) = b.x {
                        f.x_range = clamp(f.x_range, x);
                    }
                    if let Some(a) = b.a {
                        f.a_range = clamp(f.a_range, a);
                    }
                }
                Operation::RotaryRough(r) => {
                    if let Some(x) = b.x {
                        r.x_range = clamp(r.x_range, x);
                    }
                    if b.a.is_some() {
                        return Err(format!("operation {:?}: roughing goes all the way round; an angular range does not apply", o.id));
                    }
                }
                _ => {
                    return Err(format!(
                        "operation {:?} ({}): takes its geometry from its parameters, not from a selection",
                        o.id,
                        crate::ops::kind_of(&o.op)
                    ))
                }
            }
        }
        Ok(job)
    }

    fn setup_job_raw(&self, setup: &Setup, libraries: &[ToolLibrary]) -> Result<Job, String> {
        let mut part = serde_json::Map::new();
        for id in &setup.part {
            let m = self
                .models
                .iter()
                .find(|m| &m.id == id)
                .ok_or(format!("unknown model {id:?}"))?;
            match &m.kind {
                ModelKind::Profile(p) => {
                    part.insert(
                        "profile".into(),
                        serde_json::to_value(p).map_err(|e| e.to_string())?,
                    );
                }
                ModelKind::Mesh(src) => {
                    part.insert(
                        "mesh".into(),
                        serde_json::to_value(src).map_err(|e| e.to_string())?,
                    );
                }
            }
        }
        let programs: Vec<Value> = setup
            .outputs
            .iter()
            .filter_map(|out| {
                let ops: Vec<&Operation> = out
                    .operations
                    .iter()
                    .filter_map(|id| setup.operations.iter().find(|o| &o.id == id))
                    .filter(|o| o.enabled)
                    .map(|o| &o.op)
                    .collect();
                if ops.is_empty() {
                    return None;
                }
                Some(serde_json::json!({
                    "name": out.name, "title": out.title, "notes": out.notes,
                    "reset_a": out.reset_a, "operations": ops,
                }))
            })
            .collect();
        let stock = setup.stock.cylinder();
        let mut v = serde_json::json!({
            "schema_version": job::SCHEMA_VERSION,
            "stock": stock,
            "clearance": setup.clearance,
            "part": part,
            "tools": self.tools,
            "spindle_rpm": setup.spindle_rpm,
            "programs": programs,
        });
        if let Some(m) = &setup.machine {
            v["machine"] = Value::String(m.clone());
        }
        profiles::resolve_job_tools(&mut v, libraries)?;
        serde_json::from_value(v).map_err(|e| format!("setup {:?}: {e}", setup.id))
    }

    /// Project operation ids in planning order for a setup (matches `Move::Operation` ids).
    pub fn planned_op_ids(&self, setup: &Setup) -> Vec<String> {
        setup
            .outputs
            .iter()
            .flat_map(|out| out.operations.iter())
            .filter(|id| setup.operations.iter().any(|o| &o.id == *id && o.enabled))
            .cloned()
            .collect()
    }
}

/// Result of simulating a whole project: setups in order on one simulated bar.
#[derive(Debug, Clone)]
pub struct ProjectSim {
    /// Stock after each setup: `(setup id, stock)`.
    pub after: Vec<(String, crate::sim::RadialMap)>,
    pub part: crate::sim::Part,
    /// Rapids that went through stock, per setup.
    pub rapid_hits: Vec<(String, usize)>,
}

/// Simulate every setup in order. A setup whose stock is `from_previous` starts from the
/// previous setup's result (same work origin); otherwise it starts from fresh bar stock. The
/// grid covers the first setup's part (as [`crate::simulate_job`] does).
pub fn simulate_project(
    p: &Project,
    jobs: &[(String, Job)],
    mesh_for: &dyn Fn(&Job) -> Option<crate::Mesh>,
    dx: f64,
    na: usize,
) -> Result<ProjectSim, String> {
    let (_, first) = jobs.first().ok_or("the project has no setups")?;
    let mesh0 = mesh_for(first);
    let base =
        crate::simulate_job(first, &[], mesh0.as_ref(), dx, na, 0.02).map_err(|e| e.to_string())?;
    let grid = base.stock.grid;
    let mut stock: Option<crate::sim::RadialMap> = None;
    let mut after = Vec::new();
    let mut rapid_hits = Vec::new();
    for (id, job) in jobs {
        let setup = p.setup(id).ok_or(format!("unknown setup {id:?}"))?;
        let start = match (&setup.stock, &stock) {
            (StockDef::FromPrevious { .. }, Some(prev)) => prev.clone(),
            (StockDef::FromPrevious { .. }, None) => {
                return Err(format!(
                    "setup {id:?}: stock from_previous, but it is the first setup"
                ))
            }
            (StockDef::Cylinder { radius, .. }, _) => crate::sim::RadialMap::filled(grid, *radius),
        };
        let mesh = mesh_for(job);
        let tps = crate::plan_with_mesh(job, mesh.as_ref()).map_err(|e| e.to_string())?;
        let cutters = |t: u32| job.tool(t).and_then(|d| crate::cutter_for(t, d.shape).ok());
        let mut s = start;
        let mut hits = 0;
        for tp in &tps {
            hits += crate::sim::simulate(&mut s, tp, &cutters, (dx * 2.0).min(0.1))
                .rapid_hits
                .len();
        }
        rapid_hits.push((id.clone(), hits));
        after.push((id.clone(), s.clone()));
        stock = Some(s);
    }
    Ok(ProjectSim {
        after,
        part: base.part,
        rapid_hits,
    })
}

/// Jobs to plan from a 0.1 job or 0.2 project text: one per setup, as `(setup id, job)`.
pub fn jobs_from_any(text: &str, libraries: &[ToolLibrary]) -> Result<Vec<(String, Job)>, String> {
    jobs_from_any_with(text, libraries, &|_| None)
}

/// Like [`jobs_from_any`], with mesh models available (for face selections).
pub fn jobs_from_any_with(
    text: &str,
    libraries: &[ToolLibrary],
    meshes: &dyn Fn(&MeshSource) -> Option<crate::Mesh>,
) -> Result<Vec<(String, Job)>, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
    if v.get("schema_version").and_then(Value::as_str) == Some(job::SCHEMA_VERSION) {
        return Ok(vec![(
            "setup1".into(),
            profiles::job_from_json(text, libraries)?,
        )]);
    }
    let (p, _) = Project::from_json_any(text)?;
    let by_model = |id: &str| match &p.models.iter().find(|m| m.id == id)?.kind {
        ModelKind::Mesh(src) => meshes(src),
        ModelKind::Profile(_) => None,
    };
    p.setups
        .iter()
        .map(|s| Ok((s.id.clone(), p.setup_job_with(s, libraries, &by_model)?)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KNOB: &str = include_str!("../../../examples/knob/knob.job.json");

    #[test]
    fn upgrade_round_trip_and_lowering() {
        let (p, upgraded) = Project::from_json_any(KNOB).unwrap();
        assert!(upgraded);
        assert_eq!(p.setups.len(), 1);
        let s = &p.setups[0];
        assert_eq!(s.operations.len(), 3);
        assert_eq!(s.outputs[1].operations, vec!["op2", "op3"]);
        // 0.2 JSON round-trips
        let text = serde_json::to_string_pretty(&p).unwrap();
        let (back, up2) = Project::from_json_any(&text).unwrap();
        assert!(!up2);
        assert_eq!(back, p);
        // lowering gives back the original job
        let job = p.setup_job(s, &[]).unwrap();
        let orig = Job::from_json(KNOB).unwrap();
        assert_eq!(job, orig);
        assert_eq!(p.planned_op_ids(s), vec!["op1", "op2", "op3"]);
    }

    #[test]
    fn disabled_operations_and_empty_outputs_drop_out() {
        let (mut p, _) = Project::from_json_any(KNOB).unwrap();
        p.setups[0].operations[1].enabled = false; // knurl
        let job = p.setup_job(&p.setups[0], &[]).unwrap();
        assert_eq!(job.programs[1].operations.len(), 1);
        assert_eq!(p.planned_op_ids(&p.setups[0]), vec!["op1", "op3"]);
        p.setups[0].operations[2].enabled = false;
        let job = p.setup_job(&p.setups[0], &[]).unwrap();
        assert_eq!(job.programs.len(), 1);
    }

    #[test]
    fn validation_errors() {
        let (p, _) = Project::from_json_any(KNOB).unwrap();
        let mut bad = p.clone();
        bad.setups[0].outputs[0].operations.push("nope".into());
        assert!(bad
            .validate()
            .unwrap_err()
            .contains("unknown operation \"nope\""));
        let mut bad = p.clone();
        bad.setups[0].operations[1].id = "op1".into();
        assert!(bad.validate().unwrap_err().contains("used twice"));
        let mut bad = p.clone();
        bad.setups[0].part.push("ghost".into());
        assert!(bad.validate().unwrap_err().contains("unknown model"));
        assert!(Project::from_json_any("{\"schema_version\": \"9\"}").is_err());
    }

    /// The knob as two setups: turning on fresh bar, then knurl + chamfer on the turned stock.
    pub(crate) fn two_setups() -> Project {
        let (mut p, _) = Project::from_json_any(KNOB).unwrap();
        let mut s2 = p.setups[0].clone();
        let s1 = &mut p.setups[0];
        s1.operations.truncate(1);
        s1.outputs.truncate(1);
        s2.id = "setup2".into();
        s2.name = "Knurl (re-chucked)".into();
        s2.stock = StockDef::FromPrevious {
            radius: 12.5,
            length: 100.0,
        };
        s2.operations.remove(0);
        s2.outputs.remove(0);
        p.setups.push(s2);
        p
    }

    #[test]
    fn two_setups_plan_like_one_and_carry_the_stock() {
        let p = two_setups();
        p.validate().unwrap();
        let text = serde_json::to_string(&p).unwrap();
        let jobs = jobs_from_any(&text, &[]).unwrap();
        assert_eq!(jobs.len(), 2);
        // each setup posts exactly the program it had in the single-setup job
        let one = Job::from_json(KNOB).unwrap();
        let m = crate::Machine::makera_z1();
        let single: Vec<String> = crate::plan(&one)
            .unwrap()
            .iter()
            .map(|t| crate::post(t, &m))
            .collect();
        let split: Vec<String> = jobs
            .iter()
            .flat_map(|(_, j)| crate::plan(j).unwrap())
            .map(|t| crate::post(&t, &m))
            .collect();
        assert!(single == split, "G-code differs when split into setups");
        // the second setup cuts the stock the first one left
        let carried = simulate_project(&p, &jobs, &|_| None, 0.2, 90).unwrap();
        let (one_p, _) = Project::from_json_any(KNOB).unwrap();
        let one_jobs = jobs_from_any(KNOB, &[]).unwrap();
        let whole = simulate_project(&one_p, &one_jobs, &|_| None, 0.2, 90).unwrap();
        assert_eq!(
            carried.after[1].1.r, whole.after[0].1.r,
            "carried stock differs from one setup"
        );
        // ... and from fresh bar it would not match
        let mut fresh = p.clone();
        fresh.setups[1].stock = StockDef::Cylinder {
            radius: 12.5,
            length: 100.0,
        };
        let f = simulate_project(&fresh, &jobs, &|_| None, 0.2, 90).unwrap();
        assert_ne!(f.after[1].1.r, whole.after[0].1.r);
        assert_eq!(
            carried.rapid_hits,
            vec![("setup1".into(), 0), ("setup2".into(), 0)]
        );
        // a first setup cannot carry stock; output names must be unique
        let mut bad = p.clone();
        bad.setups.swap(0, 1);
        assert!(bad.validate().unwrap_err().contains("first setup"));
        let mut bad = p.clone();
        bad.setups[1].outputs[0].name = "knob_1_profile".into();
        assert!(bad.validate().unwrap_err().contains("used twice"));
    }

    #[test]
    fn face_selection_limits_where_the_tool_goes() {
        let text = include_str!("../../../examples/knob/knob_finish.job.json");
        let (mut p, _) = Project::from_json_any(text).unwrap();
        // finish only the taper of the revolved profile (the mesh is the same shape)
        let regions = p.model_regions("profile", &|_| None).unwrap();
        let taper = regions
            .iter()
            .find(|r| r.shape == "cone" && (r.x[0] + 15.0).abs() < 1e-6)
            .unwrap();
        let s = &mut p.setups[0];
        let finish = s.operations.iter_mut().find(|o| o.id == "op2").unwrap();
        finish.geometry = vec![crate::geometry::Selection::Faces {
            model: "profile".into(),
            ids: vec![taper.id.clone()],
        }];
        let text = serde_json::to_string(&p).unwrap();
        let (p, _) = Project::from_json_any(&text).unwrap();
        let job = p.setup_job(&p.setups[0], &[]).unwrap();
        let Operation::RotaryFinish(f) = &job.programs[1].operations[0] else {
            panic!()
        };
        // ball 3.175: the taper's X extent grown by the tool radius, inside the op's own
        // range [-21.0, 0.6]; the planned cuts stay inside it
        // plan against the revolved profile (the mesh file is the same shape)
        let mut no_mesh = job.clone();
        no_mesh.part.mesh = None;
        let tps = crate::plan(&no_mesh).unwrap();
        let (lo, hi) = tps[1]
            .moves
            .iter()
            .filter_map(|m| match m {
                crate::Move::Cut { x, .. } => Some(*x),
                _ => None,
            })
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), x| {
                (a.min(x), b.max(x))
            });
        assert!(lo >= -16.5876 && hi <= 0.6001, "cuts span {lo}..{hi}");
        assert!(
            (f.x_range[0] - (-15.0 - 1.5875)).abs() < 1e-9 && (f.x_range[1] - 0.6).abs() < 1e-9,
            "{:?}",
            f.x_range
        );
        // an axial range on the knurl is refused: it takes its geometry from its parameters
        let (mut q, _) = Project::from_json_any(KNOB).unwrap();
        q.setups[0].operations[1].geometry =
            vec![crate::geometry::Selection::AxialRange { x: [-5.0, 0.0] }];
        assert!(q
            .setup_job(&q.setups[0], &[])
            .unwrap_err()
            .contains("takes its geometry from its parameters"));
    }

    #[test]
    fn library_tools_resolve_when_lowering() {
        let (mut p, _) = Project::from_json_any(KNOB).unwrap();
        p.tools[0] = ToolEntry::Library {
            number: 1,
            library: "makera-starter".into(),
            tool: "spiral-o-metal-3.175x12".into(),
            description: None,
        };
        let text = serde_json::to_string(&p).unwrap();
        let (p2, _) = Project::from_json_any(&text).unwrap();
        assert!(matches!(p2.tools[0], ToolEntry::Library { .. }));
        let job = p2
            .setup_job(&p2.setups[0], &profiles::builtin_libraries())
            .unwrap();
        assert_eq!(
            job.tools[0].description,
            "Spiral O Metal 3.175 x 12 mm (single flute)"
        );
    }
}
