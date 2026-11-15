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
        for s in &self.setups {
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

    pub fn setup(&self, id: &str) -> Option<&Setup> {
        self.setups.iter().find(|s| s.id == id)
    }

    /// The 0.1 job that plans a setup (enabled operations only; empty outputs dropped).
    pub fn setup_job(&self, setup: &Setup, libraries: &[ToolLibrary]) -> Result<Job, String> {
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

/// Jobs to plan from a 0.1 job or 0.2 project text: one per setup, as `(setup id, job)`.
pub fn jobs_from_any(text: &str, libraries: &[ToolLibrary]) -> Result<Vec<(String, Job)>, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
    if v.get("schema_version").and_then(Value::as_str) == Some(job::SCHEMA_VERSION) {
        return Ok(vec![(
            "setup1".into(),
            profiles::job_from_json(text, libraries)?,
        )]);
    }
    let (p, _) = Project::from_json_any(text)?;
    p.setups
        .iter()
        .map(|s| Ok((s.id.clone(), p.setup_job(s, libraries)?)))
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
