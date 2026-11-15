//! 4cam: a 4th-axis (rotary) CAM engine.
//!
//! Pipeline: [`Job`] (JSON) -> [`plan`] -> machine-neutral [`Toolpath`]s -> [`post`] with a
//! [`Machine`] profile -> G-code, which [`verify_gcode`] can sanity-check.

pub mod geom;
pub mod job;
pub mod machine;
pub mod numeric;
pub mod post;
pub mod strategy;
pub mod toolpath;
pub mod verify;

pub use job::Job;
pub use machine::{FeedMode, Machine};
pub use toolpath::{Move, Toolpath};
pub use verify::{Issue, IssueKind, Limits};

use geom::RevolvedProfile;
use job::{Operation, Program, ToolShape};
use std::fmt;
use toolpath::Builder;

/// Errors found while planning a job.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    UnsupportedSchema(String),
    UnknownTool {
        program: String,
        tool: u32,
    },
    WrongToolShape {
        program: String,
        tool: u32,
        operation: &'static str,
        needs: &'static str,
    },
    FluteTooShort {
        tool: u32,
        depth: f64,
        flute_length: f64,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnsupportedSchema(v) => write!(
                f,
                "unsupported schema_version {v:?} (expected {:?})",
                job::SCHEMA_VERSION
            ),
            Error::UnknownTool { program, tool } => {
                write!(f, "program {program}: tool T{tool} is not defined")
            }
            Error::WrongToolShape {
                program,
                tool,
                operation,
                needs,
            } => write!(
                f,
                "program {program}: {operation} needs a {needs} tool, T{tool} is not one"
            ),
            Error::FluteTooShort {
                tool,
                depth,
                flute_length,
            } => write!(
                f,
                "T{tool}: radial depth {depth:.3} mm exceeds usable flute length ({flute_length} mm - 1 mm)"
            ),
        }
    }
}

impl std::error::Error for Error {}

fn op_name(op: &Operation) -> &'static str {
    match op {
        Operation::RotaryTurning(_) => "rotary_turning",
        Operation::HelicalKnurl(_) => "helical_knurl",
        Operation::FlankChamfer(_) => "flank_chamfer",
    }
}

fn plan_program(job: &Job, prog: &Program, geom: &RevolvedProfile) -> Result<Toolpath, Error> {
    let safe_r = job.safe_r();
    let mut b = Builder::new();
    if prog.reset_a {
        b.reset_a();
    }
    let mut current: Option<u32> = None;
    let mut first_tool = None;
    for op in &prog.operations {
        let number = op.tool();
        let tool = job.tool(number).ok_or_else(|| Error::UnknownTool {
            program: prog.name.clone(),
            tool: number,
        })?;
        let wrong = |needs| Error::WrongToolShape {
            program: prog.name.clone(),
            tool: number,
            operation: op_name(op),
            needs,
        };
        // validate before emitting anything
        match (op, tool.shape) {
            (Operation::RotaryTurning(_), ToolShape::Flat { .. }) => {
                if let Some(flute) = tool.flute_length {
                    let depth = job.stock.radius - geom.min_radius();
                    if depth >= flute - 1.0 {
                        return Err(Error::FluteTooShort {
                            tool: number,
                            depth,
                            flute_length: flute,
                        });
                    }
                }
            }
            (Operation::RotaryTurning(_), _) => return Err(wrong("flat")),
            (Operation::HelicalKnurl(_) | Operation::FlankChamfer(_), ToolShape::Vbit { .. }) => {}
            _ => return Err(wrong("vbit")),
        }
        if current != Some(number) {
            b.tool_change(number, job.spindle_rpm);
            b.rapid_r(safe_r);
            current = Some(number);
            first_tool.get_or_insert((number, tool.description.clone()));
        }
        match (op, tool.shape) {
            (Operation::RotaryTurning(o), ToolShape::Flat { diameter }) => {
                strategy::rotary_turning(&mut b, geom, diameter, job.stock.radius, safe_r, o)
            }
            (Operation::HelicalKnurl(o), ToolShape::Vbit { angle }) => {
                strategy::helical_knurl(&mut b, angle, safe_r, o)
            }
            (Operation::FlankChamfer(o), ToolShape::Vbit { angle }) => {
                strategy::flank_chamfer(&mut b, angle, safe_r, o)
            }
            _ => unreachable!("tool shape validated above"),
        }
    }
    Ok(Toolpath {
        name: prog.name.clone(),
        title: prog.title.clone(),
        tool: first_tool,
        notes: prog.notes.clone(),
        reset_a: prog.reset_a,
        safe_r,
        moves: b.moves,
    })
}

/// Plan every program of the job.
pub fn plan(job: &Job) -> Result<Vec<Toolpath>, Error> {
    if job.schema_version != job::SCHEMA_VERSION {
        return Err(Error::UnsupportedSchema(job.schema_version.clone()));
    }
    let geom = RevolvedProfile::new(&job.part.profile, job.stock.radius);
    job.programs
        .iter()
        .map(|p| plan_program(job, p, &geom))
        .collect()
}

/// Post one toolpath to G-code for `machine`.
pub fn post(tp: &Toolpath, machine: &Machine) -> String {
    post::post(tp, machine)
}

/// Check G-code text against the limits.
pub fn verify_gcode(text: &str, limits: &Limits) -> Vec<Issue> {
    verify::verify(text, limits)
}

/// Limits matching a planned toolpath: its safe radius and slowest plunge feed.
pub fn limits_for(tp: &Toolpath, machine: &Machine) -> Limits {
    let plunge = tp
        .moves
        .iter()
        .filter_map(|m| match m {
            Move::Plunge { feed, .. } => Some(*feed),
            Move::Cut { plunge_cap, .. } => Some(*plunge_cap),
            _ => None,
        })
        .fold(f64::INFINITY, f64::min);
    Limits {
        plunge_feed: if plunge.is_finite() { plunge } else { 60.0 },
        safe_r: tp.safe_r,
        feed_mode: machine.feed_mode,
    }
}
