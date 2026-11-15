//! 4cam: a 4th-axis (rotary) CAM engine.
//!
//! Pipeline: [`Job`] (JSON) -> [`plan`] -> machine-neutral [`Toolpath`]s -> [`post`] with a
//! [`Machine`] profile -> G-code, which [`verify_gcode`] can sanity-check.

pub mod archive;
pub mod dropcutter;
pub mod geom;
pub mod geometry;
pub mod job;
pub mod machine;
pub mod mesh;
pub mod numeric;
pub mod ops;
pub mod post;
pub mod profiles;
pub mod project;
pub mod render;
pub mod sim;
pub mod strategy;
pub mod toolpath;
pub mod verify;
pub mod wizards;

pub use job::Job;
pub use machine::{FeedMode, Machine};
pub use mesh::Mesh;
pub use toolpath::{Move, Toolpath};
pub use verify::{Issue, IssueKind, Limits};

use dropcutter::{Cutter, DropCutter};
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
    /// Anything else wrong with the job.
    Invalid(String),
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
            Error::Invalid(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for Error {}

fn op_name(op: &Operation) -> &'static str {
    match op {
        Operation::RotaryTurning(_) => "rotary_turning",
        Operation::HelicalKnurl(_) => "helical_knurl",
        Operation::FlankChamfer(_) => "flank_chamfer",
        Operation::RotaryFinish(_) => "rotary_finish",
        Operation::RotaryRough(_) => "rotary_rough",
    }
}

/// Facets around the axis when a profile is revolved into a finishing mesh.
pub const PROFILE_SEGMENTS: usize = 720;

/// Planning context shared by the programs of a job.
struct Ctx<'a> {
    job: &'a Job,
    profile: Option<RevolvedProfile>,
    mesh: Option<&'a Mesh>,
    revolved: Option<Mesh>,
}

impl Ctx<'_> {
    /// Finishing surface: the loaded mesh, else the revolved profile.
    fn surface(&mut self, program: &str) -> Result<&Mesh, Error> {
        if let Some(m) = self.mesh {
            return Ok(m);
        }
        if self.job.part.mesh.is_some() {
            return Err(Error::Invalid(format!(
                "program {program}: part.mesh is set but the mesh was not loaded (see load_mesh)"
            )));
        }
        let profile = self.job.part.profile.as_ref().ok_or_else(|| {
            Error::Invalid(format!(
                "program {program}: mesh operations need part.mesh or part.profile"
            ))
        })?;
        Ok(self
            .revolved
            .get_or_insert_with(|| Mesh::revolve(profile, PROFILE_SEGMENTS)))
    }
}

fn check_flute(tool: &job::Tool, stock_r: f64, r_min: f64) -> Result<(), Error> {
    if let Some(flute) = tool.flute_length {
        let depth = stock_r - r_min;
        if depth >= flute - 1.0 {
            return Err(Error::FluteTooShort {
                tool: tool.number,
                depth,
                flute_length: flute,
            });
        }
    }
    Ok(())
}

pub fn cutter_for(number: u32, shape: ToolShape) -> Result<Cutter, Error> {
    Ok(match shape {
        ToolShape::Flat { diameter } => Cutter::Flat {
            radius: diameter / 2.0,
        },
        ToolShape::Ball { diameter } => Cutter::Ball {
            radius: diameter / 2.0,
        },
        ToolShape::Vbit {
            angle,
            diameter: Some(d),
        } => Cutter::Cone {
            radius: d / 2.0,
            half_angle: (angle / 2.0).to_radians(),
        },
        ToolShape::Vbit { diameter: None, .. } => {
            return Err(Error::Invalid(format!(
                "T{number}: rotary_finish with a V-bit needs the tool's diameter"
            )))
        }
    })
}

fn plan_program(ctx: &mut Ctx, prog: &Program, next_op: &mut u32) -> Result<Toolpath, Error> {
    let job = ctx.job;
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
        let cutter = match (op, tool.shape) {
            (Operation::RotaryTurning(_), ToolShape::Flat { .. }) => {
                let geom = ctx.profile.as_ref().ok_or_else(|| {
                    Error::Invalid(format!(
                        "program {}: rotary_turning needs part.profile",
                        prog.name
                    ))
                })?;
                check_flute(tool, job.stock.radius, geom.min_radius())?;
                None
            }
            (Operation::RotaryTurning(_), _) => return Err(wrong("flat")),
            (Operation::HelicalKnurl(_) | Operation::FlankChamfer(_), ToolShape::Vbit { .. }) => {
                None
            }
            (Operation::HelicalKnurl(_) | Operation::FlankChamfer(_), _) => {
                return Err(wrong("vbit"))
            }
            (Operation::RotaryFinish(_) | Operation::RotaryRough(_), shape) => {
                Some(cutter_for(number, shape)?)
            }
        };
        b.operation(*next_op, op_name(op));
        *next_op += 1;
        if current != Some(number) {
            b.tool_change(number, job.spindle_rpm);
            b.rapid_r(safe_r);
            current = Some(number);
            first_tool.get_or_insert((number, tool.description.clone()));
        }
        match (op, tool.shape) {
            (Operation::RotaryTurning(o), ToolShape::Flat { diameter }) => {
                let geom = ctx.profile.as_ref().expect("checked above");
                strategy::rotary_turning(&mut b, geom, diameter, job.stock.radius, safe_r, o)
            }
            (Operation::HelicalKnurl(o), ToolShape::Vbit { angle, .. }) => {
                strategy::helical_knurl(&mut b, angle, safe_r, o)
            }
            (Operation::FlankChamfer(o), ToolShape::Vbit { angle, .. }) => {
                strategy::flank_chamfer(&mut b, angle, safe_r, o)
            }
            (Operation::RotaryFinish(o), _) => {
                let mesh = ctx.surface(&prog.name)?;
                let dc = DropCutter::new(mesh, cutter.expect("set above"));
                let r_min = strategy::rotary_finish(&mut b, &dc, mesh.max_radius(), safe_r, o)
                    .map_err(|e| Error::Invalid(format!("program {}: {e}", prog.name)))?;
                check_flute(tool, job.stock.radius, r_min)?;
            }
            (Operation::RotaryRough(o), _) => {
                let mesh = ctx.surface(&prog.name)?;
                let dc = DropCutter::new(mesh, cutter.expect("set above"));
                let r_min = strategy::rotary_rough(&mut b, &dc, job.stock.radius, safe_r, o)
                    .map_err(|e| Error::Invalid(format!("program {}: {e}", prog.name)))?;
                check_flute(tool, job.stock.radius, r_min)?;
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

/// Plan every program of the job. Jobs with `part.mesh` need [`plan_with_mesh`].
pub fn plan(job: &Job) -> Result<Vec<Toolpath>, Error> {
    plan_with_mesh(job, None)
}

/// Plan every program, finishing against `mesh` (already oriented, see [`load_mesh`]).
pub fn plan_with_mesh(job: &Job, mesh: Option<&Mesh>) -> Result<Vec<Toolpath>, Error> {
    if job.schema_version != job::SCHEMA_VERSION {
        return Err(Error::UnsupportedSchema(job.schema_version.clone()));
    }
    let mut ctx = Ctx {
        job,
        profile: job
            .part
            .profile
            .as_ref()
            .map(|p| RevolvedProfile::new(p, job.stock.radius)),
        mesh,
        revolved: None,
    };
    let mut next_op = 0;
    job.programs
        .iter()
        .map(|p| plan_program(&mut ctx, p, &mut next_op))
        .collect()
}

/// Load and orient the job's `part.mesh` (path relative to `base_dir`), if any.
pub fn load_mesh(job: &Job, base_dir: &std::path::Path) -> Result<Option<Mesh>, Error> {
    let Some(src) = &job.part.mesh else {
        return Ok(None);
    };
    let mesh = Mesh::load(&base_dir.join(&src.file)).map_err(|e| Error::Invalid(e.to_string()))?;
    Ok(Some(mesh.oriented(src.axis, src.translate)))
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

/// Gouge check of one program against the finished part.
#[derive(Debug, Clone, PartialEq)]
pub struct GougeReport {
    pub program: String,
    /// Cut moves checked (moves of tools used by turning, roughing and finishing operations;
    /// knurling and chamfering cut into the part on purpose and are skipped).
    pub moves_checked: usize,
    /// Deepest cut into the part: the smaller of the vertical depth and the axial distance
    /// to where the tool would clear the part (mm).
    pub max_depth: f64,
    /// Where: `(x, a)`.
    pub at: Option<(f64, f64)>,
}

/// Check every program against the part surface (the mesh if given, else the revolved
/// profile) with a drop-cutter for each tool.
pub fn gouge_check(
    job: &Job,
    tps: &[Toolpath],
    mesh: Option<&Mesh>,
) -> Result<Vec<GougeReport>, Error> {
    let revolved;
    let surface = match (mesh, &job.part.profile) {
        (Some(m), _) => m,
        (None, Some(p)) => {
            revolved = Mesh::revolve(p, PROFILE_SEGMENTS);
            &revolved
        }
        (None, None) => return Ok(Vec::new()),
    };
    let mut cutters: std::collections::HashMap<u32, DropCutter> = Default::default();
    let mut out = Vec::new();
    for (tp, prog) in tps.iter().zip(&job.programs) {
        let checked: Vec<u32> = prog
            .operations
            .iter()
            .filter(|o| {
                matches!(
                    o,
                    Operation::RotaryTurning(_)
                        | Operation::RotaryFinish(_)
                        | Operation::RotaryRough(_)
                )
            })
            .map(Operation::tool)
            .collect();
        let mut rep = GougeReport {
            program: tp.name.clone(),
            moves_checked: 0,
            max_depth: 0.0,
            at: None,
        };
        let (mut x, mut r, mut a) = (None::<f64>, None::<f64>, 0.0);
        let mut tool: Option<u32> = None;
        for m in &tp.moves {
            match *m {
                Move::ToolChange { tool: t, .. } => {
                    tool = checked.contains(&t).then_some(t);
                    if let (Some(t), Some(def)) = (tool, job.tool(t)) {
                        if let (Ok(c), std::collections::hash_map::Entry::Vacant(e)) =
                            (cutter_for(t, def.shape), cutters.entry(t))
                        {
                            e.insert(DropCutter::new(surface, c));
                        }
                    }
                }
                Move::Rapid {
                    x: nx,
                    r: nr,
                    a: na,
                } => {
                    x = nx.or(x);
                    r = nr.or(r);
                    a = na.unwrap_or(a);
                }
                Move::Plunge { r: nr, .. } => r = Some(nr),
                Move::Cut {
                    x: nx,
                    r: nr,
                    a: na,
                    ..
                } => {
                    if let (Some(dc), Some(x0), Some(r0)) =
                        (tool.and_then(|t| cutters.get(&t)), x, r)
                    {
                        rep.moves_checked += 1;
                        for k in 1..=4 {
                            let t = k as f64 / 4.0;
                            let (xi, ri, ai) =
                                (x0 + (nx - x0) * t, r0 + (nr - r0) * t, a + (na - a) * t);
                            let deficit = dc.radius_at(xi, ai, 0.0) - ri;
                            if deficit <= rep.max_depth {
                                continue;
                            }
                            // axial distance to where the tool clears the part (walls)
                            let mut depth = deficit;
                            for s in 1..=200 {
                                let d = s as f64 * 0.0005;
                                if dc.radius_at(xi - d, ai, 0.0) <= ri
                                    || dc.radius_at(xi + d, ai, 0.0) <= ri
                                {
                                    depth = depth.min(d);
                                    break;
                                }
                            }
                            if depth > rep.max_depth {
                                rep.max_depth = depth;
                                rep.at = Some((xi, ai));
                            }
                        }
                    }
                    x = Some(nx);
                    r = Some(nr);
                    a = na;
                }
                Move::Comment { .. } | Move::Operation { .. } => {}
            }
        }
        out.push(rep);
    }
    Ok(out)
}

/// Simulation of a whole job on a radial heightmap, programs in order on the same stock.
#[derive(Debug, Clone)]
pub struct JobSim {
    pub stock: sim::RadialMap,
    pub part: sim::Part,
    /// Stock after each program, with its statistics.
    pub programs: Vec<(String, sim::SimStats, sim::Deviation)>,
}

/// Simulate `tps` (from [`plan_with_mesh`]) on stock of `job.stock.radius`. The grid spans the
/// part's X range plus 3 mm each side with cells `dx` mm long and `na` around. Deviations are
/// measured over the part's X range; `tol` is the gouge threshold (mm).
pub fn simulate_job(
    job: &Job,
    tps: &[Toolpath],
    mesh: Option<&Mesh>,
    dx: f64,
    na: usize,
    tol: f64,
) -> Result<JobSim, Error> {
    let (x_lo, x_hi, part_fn): (f64, f64, Box<dyn Fn(sim::Grid) -> sim::Part>) =
        match (mesh, &job.part.profile) {
            (Some(m), _) => {
                let (lo, hi) = m
                    .bounds()
                    .ok_or_else(|| Error::Invalid("empty mesh".into()))?;
                (lo[0], hi[0], Box::new(move |g| sim::Part::from_mesh(g, m)))
            }
            (None, Some(p)) => {
                let geom = RevolvedProfile::new(p, job.stock.radius);
                (
                    geom.x_min(),
                    geom.x_max(),
                    Box::new(move |g| sim::Part::from_profile(g, &geom, &p.points)),
                )
            }
            (None, None) => return Err(Error::Invalid("nothing to compare: no part".into())),
        };
    let grid = sim::Grid::new(x_lo - 3.0, x_hi + 3.0, dx, na);
    let part = part_fn(grid);
    let mut stock = sim::RadialMap::filled(grid, job.stock.radius);
    let cutters = |t: u32| job.tool(t).and_then(|d| cutter_for(t, d.shape).ok());
    let mut programs = Vec::new();
    for tp in tps {
        let st = sim::simulate(&mut stock, tp, &cutters, (dx * 2.0).min(0.1));
        let dev = sim::deviation(&stock, &part, x_lo, x_hi, tol);
        programs.push((tp.name.clone(), st, dev));
    }
    Ok(JobSim {
        stock,
        part,
        programs,
    })
}
