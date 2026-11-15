//! Machine-neutral toolpath IR. Coordinates: `x` along the rotary axis, `r` = radius of the tool
//! tip from the rotation axis (Z on the machine), `a` = rotary angle in degrees.

use serde::{Deserialize, Serialize};

/// One step of a toolpath.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "move", rename_all = "snake_case")]
pub enum Move {
    /// Operator-visible comment.
    Comment { text: String },
    /// Start of an operation: the moves until the next marker belong to it. `id` numbers the
    /// job's operations in order (0, 1, ...); `kind` is the operation type. Posts ignore it.
    Operation { id: u32, kind: String },
    /// Load a tool and start the spindle.
    ToolChange { tool: u32, rpm: u32 },
    /// Rapid; only the given axes move (machine posts r first, then x, then a).
    Rapid {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        x: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        r: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        a: Option<f64>,
    },
    /// Straight radial feed move at `feed` (mm/min).
    Plunge { r: f64, feed: f64 },
    /// Coordinated cut. `feed` is the surface feed (mm/min) along the true path; the tip never
    /// descends faster than `plunge_cap` (mm/min).
    Cut {
        x: f64,
        r: f64,
        a: f64,
        feed: f64,
        plunge_cap: f64,
    },
}

/// Toolpath of one program (one output file).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Toolpath {
    pub name: String,
    pub title: String,
    /// Tool loaded first: (number, description).
    pub tool: Option<(u32, String)>,
    pub notes: Vec<String>,
    pub reset_a: bool,
    pub safe_r: f64,
    pub moves: Vec<Move>,
}

/// True length of a move at the mean radius (minimum 0.5 mm so moves near the axis still count).
pub fn true_length(dx: f64, dr: f64, da: f64, r_from: f64, r_to: f64) -> f64 {
    let r = crate::numeric::py_max(0.5 * (r_to + r_from), 0.5);
    let arc = r * crate::numeric::rad(da);
    (dx * dx + dr * dr + arc * arc).sqrt()
}

/// Builds a toolpath while tracking the machine state.
#[derive(Debug, Clone)]
pub struct Builder {
    pub moves: Vec<Move>,
    pub x: Option<f64>,
    pub r: Option<f64>,
    pub a: f64,
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

impl Builder {
    pub fn new() -> Builder {
        Builder {
            moves: Vec::new(),
            x: None,
            r: None,
            a: 0.0,
        }
    }

    /// Re-zero the A axis (the programs start with A = 0).
    pub fn reset_a(&mut self) {
        self.a = 0.0;
    }

    pub fn comment(&mut self, text: impl Into<String>) {
        self.moves.push(Move::Comment { text: text.into() });
    }

    pub fn operation(&mut self, id: u32, kind: &str) {
        self.moves.push(Move::Operation {
            id,
            kind: kind.to_string(),
        });
    }

    pub fn tool_change(&mut self, tool: u32, rpm: u32) {
        self.moves.push(Move::ToolChange { tool, rpm });
    }

    pub fn rapid(&mut self, x: Option<f64>, r: Option<f64>, a: Option<f64>) {
        if x.is_some() {
            self.x = x;
        }
        if r.is_some() {
            self.r = r;
        }
        if let Some(a) = a {
            self.a = a;
        }
        self.moves.push(Move::Rapid { x, r, a });
    }

    pub fn rapid_r(&mut self, r: f64) {
        self.rapid(None, Some(r), None);
    }

    pub fn rapid_x(&mut self, x: f64) {
        self.rapid(Some(x), None, None);
    }

    pub fn plunge(&mut self, r: f64, feed: f64) {
        self.r = Some(r);
        self.moves.push(Move::Plunge { r, feed });
    }

    /// Feed move; dropped when its true length is below 1e-6 mm.
    pub fn cut(&mut self, x: f64, r: f64, a: f64, feed: f64, plunge_cap: f64) {
        let (x0, r0) = (
            self.x.expect("cut before X is known"),
            self.r.expect("cut before Z is known"),
        );
        if true_length(x - x0, r - r0, a - self.a, r0, r) < 1e-6 {
            return;
        }
        self.moves.push(Move::Cut {
            x,
            r,
            a,
            feed,
            plunge_cap,
        });
        self.x = Some(x);
        self.r = Some(r);
        self.a = a;
    }
}

/// Serialize toolpaths as pretty JSON (used by `4cam toolpath`).
pub fn to_json(tps: &[Toolpath]) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(tps)
}
