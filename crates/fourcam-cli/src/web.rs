//! `4cam serve`: a minimal local web UI. Std-only HTTP/1.1 server bound to 127.0.0.1.
//!
//! Routes:
//! - `GET /`             the single-page UI
//! - `GET /api/example`  the bundled knob job
//! - `GET /api/machine`  the bundled Makera Z1 profile
//! - `POST /api/run`     body `{"job": {...}, "machine": {...}?}` -> programs, G-code, issues,
//!   and a preview polyline per program

use fourcam::{limits_for, plan, post, verify_gcode, Job, Machine, Move, Toolpath};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};

const INDEX_HTML: &str = include_str!("../static/index.html");
const EXAMPLE_JOB: &str = include_str!("../../../examples/knob/knob.job.json");
const MAX_BODY: usize = 16 * 1024 * 1024;

pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: String,
}

impl Response {
    fn json(status: u16, v: &Value) -> Response {
        Response {
            status,
            content_type: "application/json",
            body: v.to_string(),
        }
    }

    fn error(status: u16, msg: impl Into<String>) -> Response {
        Response::json(status, &json!({ "error": msg.into() }))
    }
}

pub fn serve(port: u16) -> Result<(), String> {
    let addr = format!("127.0.0.1:{port}");
    let listener = TcpListener::bind(&addr).map_err(|e| format!("{addr}: {e}"))?;
    println!("4cam web UI on http://{addr}  (Ctrl+C to stop)");
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                std::thread::spawn(move || {
                    if let Err(e) = handle_conn(s) {
                        eprintln!("connection error: {e}");
                    }
                });
            }
            Err(e) => eprintln!("accept error: {e}"),
        }
    }
    Ok(())
}

fn handle_conn(stream: TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("/").to_string();
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse().unwrap_or(0);
            }
        }
    }
    let resp = if content_length > MAX_BODY {
        Response::error(413, "request body too large")
    } else {
        let mut body = vec![0u8; content_length];
        reader.read_exact(&mut body)?;
        handle(&method, &path, &String::from_utf8_lossy(&body))
    };
    write_response(stream, &resp)
}

fn write_response(mut stream: TcpStream, r: &Response) -> std::io::Result<()> {
    let reason = match r.status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        422 => "Unprocessable Entity",
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {} {reason}\r\nContent-Type: {}; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        r.status,
        r.content_type,
        r.body.len()
    )?;
    stream.write_all(r.body.as_bytes())?;
    stream.flush()
}

/// Route a request. Pure function so it can be tested without sockets.
pub fn handle(method: &str, path: &str, body: &str) -> Response {
    let path = path.split('?').next().unwrap_or(path);
    match (method, path) {
        ("GET", "/") | ("GET", "/index.html") => Response {
            status: 200,
            content_type: "text/html",
            body: INDEX_HTML.to_string(),
        },
        ("GET", "/api/example") => Response {
            status: 200,
            content_type: "application/json",
            body: EXAMPLE_JOB.to_string(),
        },
        ("GET", "/api/machine") => Response::json(
            200,
            &serde_json::to_value(Machine::makera_z1()).unwrap_or(Value::Null),
        ),
        ("POST", "/api/run") => api_run(body),
        (_, "/" | "/index.html" | "/api/example" | "/api/machine" | "/api/run") => {
            Response::error(405, "method not allowed")
        }
        _ => Response::error(404, "not found"),
    }
}

fn api_run(body: &str) -> Response {
    let req: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return Response::error(400, format!("request is not JSON: {e}")),
    };
    let job: Job = match req.get("job").map(|j| serde_json::from_value(j.clone())) {
        Some(Ok(j)) => j,
        Some(Err(e)) => return Response::error(422, format!("job: {e}")),
        None => return Response::error(400, "missing \"job\""),
    };
    let machine = match req.get("machine") {
        None | Some(Value::Null) => Machine::makera_z1(),
        Some(m) => match serde_json::from_value(m.clone()) {
            Ok(m) => m,
            Err(e) => return Response::error(422, format!("machine: {e}")),
        },
    };
    let tps = match plan(&job) {
        Ok(t) => t,
        Err(e) => return Response::error(422, e.to_string()),
    };
    let programs: Vec<Value> = tps
        .iter()
        .map(|tp| {
            let gcode = post(tp, &machine);
            let limits = limits_for(tp, &machine);
            let issues: Vec<String> = verify_gcode(&gcode, &limits)
                .iter()
                .map(|i| i.to_string())
                .collect();
            json!({
                "name": tp.name,
                "title": tp.title,
                "lines": gcode.lines().count(),
                "gcode": gcode,
                "issues": issues,
                "safe_r": tp.safe_r,
                "preview": preview(tp),
            })
        })
        .collect();
    Response::json(
        200,
        &json!({
            "stock": job.stock,
            "profile": job.part.profile,
            "programs": programs,
        }),
    )
}

/// Tool-tip polyline `[x, r, a, kind]` with kind 0 = rapid, 1 = feed. Starts once X is known.
fn preview(tp: &Toolpath) -> Vec<[f64; 4]> {
    let (mut x, mut r, mut a) = (None::<f64>, None::<f64>, 0.0);
    let mut pts = Vec::new();
    for m in &tp.moves {
        let kind = match m {
            Move::Rapid { x: nx, r: nr, a: na } => {
                x = nx.or(x);
                r = nr.or(r);
                a = na.unwrap_or(a);
                0.0
            }
            Move::Plunge { r: nr, .. } => {
                r = Some(*nr);
                1.0
            }
            Move::Cut {
                x: nx,
                r: nr,
                a: na,
                ..
            } => {
                x = Some(*nx);
                r = Some(*nr);
                a = *na;
                1.0
            }
            _ => continue,
        };
        if let (Some(x), Some(r)) = (x, r) {
            pts.push([round3(x), round3(r), round3(a), kind]);
        }
    }
    pts
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_and_example_are_served() {
        let r = handle("GET", "/", "");
        assert_eq!(r.status, 200);
        assert!(r.body.contains("<title>"));
        let r = handle("GET", "/api/example", "");
        assert_eq!(r.status, 200);
        assert!(Job::from_json(&r.body).is_ok());
        assert_eq!(handle("GET", "/api/machine", "").status, 200);
        assert_eq!(handle("GET", "/nope", "").status, 404);
        assert_eq!(handle("DELETE", "/api/run", "").status, 405);
    }

    #[test]
    fn run_plans_the_example() {
        let body = format!("{{\"job\": {EXAMPLE_JOB}}}");
        let r = handle("POST", "/api/run", &body);
        assert_eq!(r.status, 200, "{}", r.body);
        let v: Value = serde_json::from_str(&r.body).unwrap();
        let progs = v["programs"].as_array().unwrap();
        assert_eq!(progs.len(), 2);
        assert_eq!(progs[0]["lines"], 21787);
        assert_eq!(progs[0]["issues"].as_array().unwrap().len(), 0);
        assert!(!progs[1]["preview"].as_array().unwrap().is_empty());
    }

    #[test]
    fn run_reports_errors() {
        assert_eq!(handle("POST", "/api/run", "nope").status, 400);
        assert_eq!(handle("POST", "/api/run", "{}").status, 400);
        assert_eq!(handle("POST", "/api/run", "{\"job\": {}}").status, 422);
        let bad = EXAMPLE_JOB.replace("\"tool\": 1", "\"tool\": 9");
        let r = handle("POST", "/api/run", &format!("{{\"job\": {bad}}}"));
        assert_eq!(r.status, 422);
        assert!(r.body.contains("T9"));
    }
}
