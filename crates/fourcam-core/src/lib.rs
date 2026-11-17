//! 4cam: a 4th-axis (rotary) CAM engine.
//!
//! Pipeline: [`Job`] (JSON) -> [`plan`] -> machine-neutral [`Toolpath`]s -> [`post`] with a
//! [`Machine`] profile -> G-code, which [`verify_gcode`] can sanity-check.

pub mod archive;
pub mod body;
pub mod collision;
pub mod dropcutter;
pub mod engrave;
pub mod estimate;
pub mod geom;
pub mod geometry;
pub mod gpu_input;
pub mod job;
pub mod kinematics;
pub mod machine;
pub mod mesh;
pub mod numeric;
pub mod offset;
pub mod ops;
pub mod post;
pub mod profiles;
pub mod project;
pub mod relief;
pub mod render;
pub mod sim;
pub mod sim3;
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
        Operation::Face(_) => "face",
        Operation::Pocket(_) => "pocket",
        Operation::Drill(_) => "drill",
        Operation::Contour(_) => "contour",
        Operation::Raster3d(_) => "raster3d",
        Operation::Rough3d(_) => "rough3d",
        Operation::Engrave(_) => "engrave",
        Operation::Vcarve(_) => "vcarve",
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

/// The triangles of `mesh` in its face regions `ids` ([`geometry::face_regions`] at 30°,
/// the same regions a face selection names).
pub fn face_subset(mesh: &Mesh, ids: &[String]) -> Result<Mesh, String> {
    let (regions, label) = geometry::face_regions(mesh, 30.0);
    let want: Vec<usize> = ids
        .iter()
        .map(|id| {
            regions
                .iter()
                .position(|r| &r.id == id)
                .ok_or(format!("the part has no face {id:?}"))
        })
        .collect::<Result<_, _>>()?;
    Ok(Mesh {
        triangles: mesh
            .triangles
            .iter()
            .zip(&label)
            .filter(|(_, l)| want.contains(l))
            .map(|(t, _)| *t)
            .collect(),
    })
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
        ToolShape::TaperedBall {
            tip_diameter,
            taper_angle,
            diameter,
        } => {
            if !(tip_diameter > 0.0 && (0.0..90.0).contains(&taper_angle))
                || diameter < tip_diameter
            {
                return Err(Error::Invalid(format!(
                    "T{number}: a tapered ball needs tip_diameter > 0, 0 <= taper_angle < 90 deg                      (each side from the axis) and diameter >= tip_diameter"
                )));
            }
            if taper_angle < 1e-6 {
                // straight sides: a ball end mill of the tip's size
                return Ok(Cutter::Ball {
                    radius: tip_diameter / 2.0,
                });
            }
            Cutter::TaperedBall {
                tip_radius: tip_diameter / 2.0,
                half_angle: taper_angle.to_radians(),
                radius: diameter / 2.0,
            }
        }
    })
}

fn is_three_axis(op: &Operation) -> bool {
    matches!(
        op,
        Operation::Face(_)
            | Operation::Pocket(_)
            | Operation::Drill(_)
            | Operation::Contour(_)
            | Operation::Raster3d(_)
            | Operation::Rough3d(_)
    )
}

/// Turn machine-frame 3-axis moves made with the rotary at `a_deg` into general moves in
/// the part frame (`Move::Pose`); `start` is the machine position before them. Rapids that do
/// not move are dropped.
fn general_moves(moves: Vec<Move>, start: [f64; 3], a_deg: f64) -> Vec<Move> {
    let axis = kinematics::to_part([0.0, 0.0, 1.0], a_deg);
    let mut p = start;
    let mut out = Vec::with_capacity(moves.len());
    for m in moves {
        let feed = match m {
            Move::Rapid3 { x, y, z } => {
                let q = [x.unwrap_or(p[0]), y.unwrap_or(p[1]), z.unwrap_or(p[2])];
                if q == p {
                    continue;
                }
                p = q;
                None
            }
            Move::Line3 { x, y, z, feed, .. } => {
                p = [x, y, z];
                Some(feed)
            }
            other => {
                out.push(other);
                continue;
            }
        };
        out.push(Move::Pose {
            tip: kinematics::to_part(p, a_deg),
            axis,
            feed,
        });
    }
    out
}

/// Operations that work on either setup: flat on a vise, wrapped on a rotary setup.
pub fn is_surface_op(op: &Operation) -> bool {
    matches!(op, Operation::Engrave(_) | Operation::Vcarve(_))
}

fn plan_program(ctx: &mut Ctx, prog: &Program, next_op: &mut u32) -> Result<Toolpath, Error> {
    let job = ctx.job;
    // a vise setup (box stock) runs only 3-axis operations; a rotary setup runs both, the
    // 3-axis ones indexed to a fixed A (3+1)
    let three = job.stock_box.is_some();
    if three
        && prog
            .operations
            .iter()
            .any(|o| !is_three_axis(o) && !is_surface_op(o))
    {
        return Err(Error::Invalid(format!(
            "program {}: rotary operations need bar stock on the rotary axis, not a vise setup",
            prog.name
        )));
    }
    if three {
        if let Some(a) = prog
            .operations
            .iter()
            .filter_map(Operation::index_a)
            .find(|a| *a != 0.0)
        {
            return Err(Error::Invalid(format!(
                "program {}: index_a {a} on a vise setup (it has no rotary axis)",
                prog.name
            )));
        }
    }
    // the block the 3-axis strategies work in: the vise stock, or around the bar (its
    // radius in Y and Z over the part's X range) for indexed operations
    let stock_box = job.stock_box.or_else(|| {
        prog.operations.iter().any(is_three_axis).then(|| {
            let rad = job.stock.radius;
            let (xlo, xhi) = match (&ctx.profile, ctx.mesh.and_then(Mesh::bounds)) {
                (_, Some((lo, hi))) => (lo[0], hi[0]),
                (Some(p), None) => (p.x_min(), p.x_max()),
                _ => (-job.stock.length, 0.0),
            };
            job::StockBox {
                min: [xlo, -rad, -rad],
                max: [xhi, rad, rad],
            }
        })
    });
    // safe radius above the rotary axis, or safe Z above the block
    let safe_r = match (three, stock_box) {
        (true, Some(b)) => b.max[2] + job.clearance,
        _ => job.safe_r(),
    };
    let rapid_safe = |b: &mut Builder| {
        if three {
            b.rapid3(None, None, Some(safe_r));
        } else {
            b.rapid_r(safe_r);
        }
    };
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
            (
                Operation::RotaryFinish(_)
                | Operation::RotaryRough(_)
                | Operation::Raster3d(_)
                | Operation::Rough3d(_),
                shape,
            ) => Some(cutter_for(number, shape)?),
            (
                Operation::Face(_)
                | Operation::Pocket(_)
                | Operation::Drill(_)
                | Operation::Contour(_),
                ToolShape::Flat { .. },
            ) => None,
            (Operation::Engrave(_), _) => None,
            (Operation::Vcarve(_), ToolShape::Vbit { .. }) => None,
            (Operation::Vcarve(_), _) => return Err(wrong("vbit")),
            (
                Operation::Face(_)
                | Operation::Pocket(_)
                | Operation::Drill(_)
                | Operation::Contour(_),
                _,
            ) => return Err(wrong("flat")),
        };
        b.operation(*next_op, op_name(op));
        *next_op += 1;
        if current != Some(number) {
            b.tool_change(number, job.spindle_rpm);
            rapid_safe(&mut b);
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
                let c = cutter.expect("set above");
                let dc = DropCutter::new(mesh, c);
                let sel = if o.faces.is_empty() {
                    None
                } else {
                    let sub = face_subset(mesh, &o.faces)
                        .map_err(|e| Error::Invalid(format!("program {}: {e}", prog.name)))?;
                    Some(DropCutter::new(&sub, c))
                };
                let r_min = strategy::rotary_finish_on(
                    &mut b,
                    &dc,
                    sel.as_ref(),
                    mesh.max_radius(),
                    safe_r,
                    o,
                )
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
            (Operation::Engrave(_) | Operation::Vcarve(_), shape) => {
                use strategy::engrave as eg;
                let err = |e: String| Error::Invalid(format!("program {}: {e}", prog.name));
                let top_of = |t: Option<f64>| match stock_box {
                    Some(sb) if three => eg::Surface::Flat {
                        top: t.unwrap_or(sb.max[2]),
                    },
                    _ => eg::Surface::Wrapped {
                        top: t.unwrap_or(job.stock.radius),
                    },
                };
                // wrapped cuts start on the axis centreline from the rotary state
                if !three && b.p3.is_some_and(|p| p[1] != 0.0) {
                    b.rapid3(None, None, Some(safe_r));
                    b.rapid3(None, Some(0.0), None);
                }
                match op {
                    Operation::Engrave(o) => {
                        eg::engrave(&mut b, top_of(o.top), safe_r, o).map_err(err)?
                    }
                    Operation::Vcarve(o) => {
                        let ToolShape::Vbit { angle, .. } = shape else {
                            unreachable!("checked above")
                        };
                        eg::vcarve(&mut b, top_of(o.top), safe_r, (angle / 2.0).to_radians(), o)
                            .map_err(err)?
                    }
                    _ => unreachable!(),
                }
                if three {
                    b.x = None;
                } else {
                    b.p3 = None;
                }
            }
            (op3, shape) if is_three_axis(op3) => {
                let sb = stock_box.as_ref().expect("set above");
                let index = op3.index_a().unwrap_or(0.0);
                let indexed_from = b.moves.len();
                let indexed_start = [b.x.unwrap_or(sb.min[0]), 0.0, safe_r];
                if !three {
                    // indexed 3+1: up to the safe radius, turn A, then cut in X/Y/Z
                    b.rapid_r(safe_r);
                    b.rapid(None, None, Some(index));
                    b.p3 = Some(indexed_start);
                }
                let d = match shape {
                    ToolShape::Flat { diameter } | ToolShape::Ball { diameter } => diameter,
                    ToolShape::Vbit { diameter, .. } => diameter.unwrap_or(0.0),
                    ToolShape::TaperedBall { tip_diameter, .. } => tip_diameter,
                };
                let err = |e: String| Error::Invalid(format!("program {}: {e}", prog.name));
                use strategy::three_axis as t3;
                match op3 {
                    Operation::Face(o) => t3::face(&mut b, d, sb, safe_r, o),
                    Operation::Pocket(o) => t3::pocket(&mut b, d, sb, safe_r, o).map_err(err)?,
                    Operation::Drill(o) => t3::drill(&mut b, sb, safe_r, o),
                    Operation::Contour(o) => t3::contour(&mut b, d, sb, safe_r, o).map_err(err)?,
                    Operation::Raster3d(o) | Operation::Rough3d(o) => {
                        let mesh: &Mesh = if three {
                            ctx.mesh.ok_or_else(|| {
                                Error::Invalid(format!(
                                    "program {}: raster3d needs part.mesh",
                                    prog.name
                                ))
                            })?
                        } else {
                            ctx.surface(&prog.name)?
                        };
                        let turned = (index != 0.0).then(|| mesh.rotated_x(index));
                        let c = cutter.expect("set above");
                        let dc = t3::DropCutter3::new(turned.as_ref().unwrap_or(mesh), c);
                        if o.faces.is_empty() {
                            t3::raster3d(&mut b, &dc, sb, safe_r, o).map_err(err)?
                        } else {
                            let sub = face_subset(mesh, &o.faces).map_err(err)?;
                            let sub = if index != 0.0 {
                                sub.rotated_x(index)
                            } else {
                                sub
                            };
                            // the passes need cover only the faces, and the tool beside them
                            let mut o = o.clone();
                            if let Some((lo, hi)) = sub.bounds() {
                                let r = c.radius();
                                let clip = |range: [f64; 2], a: f64, b: f64| {
                                    let (l, h) = (range[0].min(range[1]), range[0].max(range[1]));
                                    let (l, h) = (l.max(a - r), h.min(b + r));
                                    if range[0] <= range[1] {
                                        [l, h]
                                    } else {
                                        [h, l]
                                    }
                                };
                                o.x_range = clip(o.x_range, lo[0], hi[0]);
                                o.y_range = clip(o.y_range, lo[1], hi[1]);
                            }
                            let seld = t3::DropCutter3::new(&sub, c);
                            t3::raster3d_on(&mut b, &dc, Some(&seld), sb, safe_r, &o)
                                .map_err(err)?
                        }
                    }
                    _ => unreachable!(),
                }
                if !three {
                    // back on the axis centreline before any rotary move
                    b.rapid3(None, None, Some(safe_r));
                    b.rapid3(None, Some(0.0), None);
                    b.x = b.p3.map(|p| p[0]);
                    b.r = Some(safe_r);
                    // the strategy worked in the machine frame at A = index: hand the post
                    // general moves (tip and tool direction in the part frame) to solve
                    let tail = b.moves.split_off(indexed_from);
                    b.moves.extend(general_moves(tail, indexed_start, index));
                }
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
        origin_comment: three.then(|| {
            "Zero: the setup's work coordinates, Z up (Z0 = top of the stock)".to_string()
        }),
        plunge_limit: prog
            .operations
            .iter()
            .any(|o| is_three_axis(o) || is_surface_op(o))
            .then(|| {
                prog.operations
                    .iter()
                    .filter_map(|op| match op {
                        Operation::Face(o) => Some(o.plunge_feed),
                        Operation::Pocket(o) => Some(o.plunge_feed),
                        Operation::Drill(o) => Some(o.feed),
                        Operation::Contour(o) => Some(o.plunge_feed),
                        Operation::Raster3d(o) | Operation::Rough3d(o) => Some(o.plunge_feed),
                        Operation::Engrave(o) => Some(o.plunge_feed),
                        Operation::Vcarve(o) => Some(o.plunge_feed),
                        _ => None,
                    })
                    .fold(f64::INFINITY, f64::min)
            })
            .filter(|v| v.is_finite()),
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
    let mesh = src
        .load(base_dir)
        .map_err(|e| Error::Invalid(e.to_string()))?;
    Ok(Some(mesh))
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
        .fold(tp.plunge_limit.unwrap_or(f64::INFINITY), f64::min);
    Limits {
        plunge_feed: if plunge.is_finite() { plunge } else { 60.0 },
        safe_r: tp.safe_r,
        feed_mode: machine.feed_mode,
        surface_radius_pad: machine.surface_radius_pad,
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
                Move::Comment { .. }
                | Move::Operation { .. }
                | Move::Rapid3 { .. }
                | Move::Line3 { .. }
                | Move::Pose { .. } => {}
            }
        }
        out.push(rep);
    }
    Ok(out)
}

/// Operation tags (id + 1, as the simulations record them) of operations meant to cut
/// below the part model: those the registry marks as shaping the part (knurls, chamfers,
/// engraving, V-carving) and, on a rotary setup, indexed 3-axis features.
pub fn intended_tags(job: &Job, tps: &[Toolpath]) -> std::collections::HashSet<u16> {
    let mut out = std::collections::HashSet::new();
    for tp in tps {
        for m in &tp.moves {
            if let Move::Operation { id, kind } = m {
                let t = ops::find(kind);
                let feature = t.is_some_and(|t| {
                    t.shapes_part || (job.stock_box.is_none() && t.machine_needs.contains(&"xyz"))
                });
                if feature {
                    out.insert((id + 1).min(u16::MAX as u32) as u16);
                }
            }
        }
    }
    out
}

/// The grid [`simulate_job`] uses for this job (the part's X range plus 3 mm each side).
pub fn sim_grid(job: &Job, mesh: Option<&Mesh>, dx: f64, na: usize) -> Result<sim::Grid, Error> {
    let (lo, hi) = match (mesh, &job.part.profile) {
        (Some(m), _) => {
            let (lo, hi) = m
                .bounds()
                .ok_or_else(|| Error::Invalid("empty mesh".into()))?;
            (lo[0], hi[0])
        }
        (None, Some(p)) => {
            let g = RevolvedProfile::new(p, job.stock.radius);
            (g.x_min(), g.x_max())
        }
        (None, None) => return Err(Error::Invalid("nothing to compare: no part".into())),
    };
    Ok(sim::Grid::new(lo - 3.0, hi + 3.0, dx, na))
}

/// Simulation of a whole job on a radial heightmap, programs in order on the same stock.
#[derive(Debug, Clone)]
pub struct JobSim {
    pub stock: sim::RadialMap,
    pub part: sim::Part,
    /// Stock after each program, with its statistics.
    pub programs: Vec<(String, sim::SimStats, sim::Deviation)>,
    /// Holder and shank collisions per program (G-code lines not filled in).
    pub collisions: Vec<collision::CollisionReport>,
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
    if job.stock_box.is_some() {
        return Err(Error::Invalid(
            "the machining simulation doesn't handle 3-axis (box stock) setups yet \
             (the engine's sim3::Heightmap3 does, for tests)"
                .into(),
        ));
    }
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
    if grid.len() > sim::MAX_CELLS {
        return Err(Error::Invalid(format!(
            "simulation grid too fine: {} cells (limit {})",
            grid.len(),
            sim::MAX_CELLS
        )));
    }
    let part = part_fn(grid);
    let mut stock = sim::RadialMap::filled(grid, job.stock.radius);
    let cutters = |t: u32| job.tool(t).and_then(|d| cutter_for(t, d.shape).ok());
    let intended = intended_tags(job, tps);
    let bodies = |t: u32| job.tool(t).map(collision::ToolBody::of);
    let bar = (job.stock.radius, -job.stock.length, 0.0);
    let mut programs = Vec::new();
    let mut collisions = Vec::new();
    for tp in tps {
        let (st, rep) = collision::simulate_rotary(
            &mut stock,
            tp,
            &cutters,
            &bodies,
            (dx * 2.0).min(0.1),
            bar,
            false,
        );
        let dev = sim::deviation_with(&stock, &part, x_lo, x_hi, tol, &|t| intended.contains(&t));
        programs.push((tp.name.clone(), st, dev));
        collisions.push(rep);
    }
    Ok(JobSim {
        stock,
        part,
        programs,
        collisions,
    })
}

/// Holder and shank collisions of each program ([`collision`]), from a material-removal
/// simulation of its own: rotary stock 0.3 mm by 2° over the whole bar, 3-axis stock at the
/// "coarse" cell size, tool positions 0.4 mm apart. It is skipped when no move brings a
/// shank or holder below the stock's top. Each collision carries its move's G-code line as
/// `machine` posts it.
pub fn collision_check(
    job: &Job,
    tps: &[Toolpath],
    machine: &Machine,
) -> Result<Vec<collision::CollisionReport>, Error> {
    let res = sim::Resolution::preset("coarse").expect("built in");
    let cutters = |t: u32| job.tool(t).and_then(|d| cutter_for(t, d.shape).ok());
    let bodies = |t: u32| job.tool(t).map(collision::ToolBody::of);
    // stamps 0.4 mm apart: the cusps left between them are a few hundredths of a mm
    let step = 0.4;
    let mut out = Vec::new();
    // nothing above the flutes ever gets below the stock's top: no simulation needed
    let (segs, top) = match &job.stock_box {
        Some(sb) => (sim3::segments3(tps), sb.max[2]),
        None => (sim::segments(tps), job.stock.radius),
    };
    if !collision::can_touch(&segs, &bodies, top) {
        return Ok(tps
            .iter()
            .map(|tp| collision::CollisionReport {
                program: tp.name.clone(),
                ..Default::default()
            })
            .collect());
    }
    if let Some(sb) = &job.stock_box {
        let mut hm = sim3::Heightmap3::new(sb, res.cell_for(sb));
        for tp in tps {
            let (_, rep) =
                collision::simulate_block(&mut hm, tp, &cutters, &bodies, step, sb.max[2]);
            out.push(rep);
        }
    } else {
        let len = job.stock.length.max(1.0);
        // 0.3 mm by 2°: holders are tens of mm across, shanks a few
        let grid = sim::Grid::new(-len - 1.0, 3.0, 0.3, 180);
        if grid.len() > sim::MAX_CELLS {
            return Err(Error::Invalid(
                "stock too long for the collision check".into(),
            ));
        }
        let mut stock = sim::RadialMap::filled(grid, job.stock.radius);
        let bar = (job.stock.radius, -len, 0.0);
        for (k, tp) in tps.iter().enumerate() {
            // the last program's stock is not needed after its last move that can touch
            let last = k + 1 == tps.len();
            let (_, rep) =
                collision::simulate_rotary(&mut stock, tp, &cutters, &bodies, step, bar, last);
            out.push(rep);
        }
    }
    for (rep, tp) in out.iter_mut().zip(tps) {
        let (_, _, lines) = post::post_lines(tp, machine);
        collision::attach_lines(rep, &lines);
    }
    Ok(out)
}
