//! `marcam mcp` over stdio, as an MCP client sees it: the handshake, the tools on the knob and
//! the plate (the knob's G-code byte-identical to the goldens), the resources, and files
//! confined to the client's roots or the `--allow` folders.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    id: u64,
    /// The roots it answers `roots/list` with (None: it has no roots capability).
    roots: Option<Vec<String>>,
}

impl Client {
    fn start(args: &[&str], roots: Option<Vec<String>>) -> Client {
        let mut child = Command::new(env!("CARGO_BIN_EXE_marcam"))
            .arg("mcp")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut c = Client {
            child,
            stdin,
            stdout,
            id: 0,
            roots,
        };
        let caps = if c.roots.is_some() {
            json!({"roots": {"listChanged": true}})
        } else {
            json!({})
        };
        let init = c.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": caps, "clientInfo": {"name": "test", "version": "1"}}),
        );
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(init["result"]["serverInfo"]["name"], "marcam");
        c.notify("notifications/initialized");
        c
    }

    fn send(&mut self, v: &Value) {
        writeln!(self.stdin, "{v}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn notify(&mut self, method: &str) {
        self.send(&json!({"jsonrpc": "2.0", "method": method}));
    }

    /// Send a request; answer the server's own requests (roots) until its response comes.
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        let id = self.id;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let mut line = String::new();
            assert!(
                self.stdout.read_line(&mut line).unwrap() > 0,
                "the server closed"
            );
            let m: Value = serde_json::from_str(&line).unwrap();
            if m["method"] == "roots/list" {
                let roots: Vec<Value> = self
                    .roots
                    .clone()
                    .unwrap_or_default()
                    .iter()
                    .map(|u| json!({"uri": u, "name": "work"}))
                    .collect();
                self.send(&json!({"jsonrpc": "2.0", "id": m["id"], "result": {"roots": roots}}));
                continue;
            }
            if m["id"] == id {
                return m;
            }
        }
    }

    /// A tool's result: Ok(structured content) or Err(the error text).
    fn tool(&mut self, name: &str, args: Value) -> Result<Value, String> {
        let r = self.request("tools/call", json!({"name": name, "arguments": args}));
        let res = &r["result"];
        if res["isError"] == true {
            return Err(res["content"][0]["text"].as_str().unwrap_or("").to_string());
        }
        Ok(res["structuredContent"].clone())
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// A scratch copy of the knob and plate examples.
fn work(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("marcam-mcp-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for ex in ["knob", "plate"] {
        let to = dir.join(ex);
        std::fs::create_dir_all(&to).unwrap();
        for f in std::fs::read_dir(root().join("examples").join(ex))
            .unwrap()
            .flatten()
        {
            if f.path().is_file() {
                std::fs::copy(f.path(), to.join(f.file_name())).unwrap();
            }
        }
    }
    dir
}

fn file_uri(p: &Path) -> String {
    let s = std::fs::canonicalize(p)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/");
    let s = s.trim_start_matches("//?/").to_string();
    if s.starts_with('/') {
        format!("file://{s}")
    } else {
        format!("file:///{s}")
    }
}

fn golden(name: &str) -> String {
    std::fs::read_to_string(root().join("tests/golden").join(name)).unwrap()
}

#[test]
fn the_protocol_tools_and_resources() {
    let dir = work("proto");
    let mut c = Client::start(&["--allow", dir.to_str().unwrap()], None);
    assert_eq!(c.request("ping", json!({}))["result"], json!({}));
    let tools: Vec<String> = c.request("tools/list", json!({}))["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        tools,
        [
            "describe",
            "open_project",
            "operation_defaults",
            "plan",
            "simulate",
            "feeds",
            "pickables",
            "replace_model"
        ]
    );
    assert_eq!(c.request("no/such", json!({}))["error"]["code"], -32601);
    // resources: the docs and the examples
    let list = c.request("resources/list", json!({}));
    let uris: Vec<&str> = list["result"]["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["uri"].as_str().unwrap())
        .collect();
    assert!(
        uris.contains(&"marcam://docs/mcp.md")
            && uris.contains(&"marcam://examples/knob.marcam.toml")
    );
    let knob = c.request(
        "resources/read",
        json!({"uri": "marcam://examples/knob.marcam.toml"}),
    );
    assert_eq!(
        knob["result"]["contents"][0]["text"].as_str().unwrap(),
        std::fs::read_to_string(root().join("examples/knob/knob.marcam.toml")).unwrap()
    );
    assert!(c.request("resources/read", json!({"uri": "marcam://nope"}))["error"].is_object());
    // describe: operations with fields, machines, tool libraries, materials
    let d = c.tool("describe", json!({})).unwrap();
    assert!(d["operations"].as_array().unwrap().len() >= 16);
    assert!(d["machines"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["id"] == "makera-z1-4axis"));
    assert!(d["tool_libraries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l["id"] == "makera-starter"));
    assert!(d["materials"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m == "aluminium"));
    let one = c
        .tool(
            "describe",
            json!({"topic": "operations", "operation": "pocket"}),
        )
        .unwrap();
    assert_eq!(one["operations"].as_array().unwrap().len(), 1);
    assert!(one.get("machines").is_none());
    // feeds
    let f = c
        .tool("feeds", json!({"material": "aluminium", "tool": {"kind": "flat", "diameter": 3.175}, "op": "pocket", "adaptive": true, "machine": "makera-carvera-4axis"}))
        .unwrap();
    assert_eq!(f["rpm"], 15000.0);
    assert!(f["feed"].as_f64().unwrap() > 500.0);
    assert!(c
        .tool(
            "feeds",
            json!({"material": "unobtainium", "tool": {"kind": "flat", "diameter": 3}})
        )
        .is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_knob_plans_to_the_goldens() {
    let dir = work("knob");
    let mut c = Client::start(&["--allow", dir.to_str().unwrap()], None);
    let p = c
        .tool("open_project", json!({"path": "knob/knob.marcam.toml"}))
        .unwrap();
    assert_eq!(p["setups"][0]["operations"].as_array().unwrap().len(), 3);
    assert_eq!(p["problems"], json!([]));
    // the G-code, all of it, and written to a folder inside the allowed one
    let r = c
        .tool(
            "plan",
            json!({"path": "knob/knob.marcam.toml", "gcode": "full", "out": "out"}),
        )
        .unwrap();
    let progs = r["setups"][0]["programs"].as_array().unwrap();
    assert_eq!(progs.len(), 2);
    for p in progs {
        let name = p["name"].as_str().unwrap();
        let want = golden(&format!("{name}.nc"));
        assert!(
            p["gcode"].as_str().unwrap() == want,
            "{name}: not the golden"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("out").join(format!("{name}.nc"))).unwrap(),
            want
        );
        assert!(p["issues"].as_array().unwrap().is_empty());
        assert!(p["time_s"].as_f64().unwrap() > 60.0);
    }
    // by default only the head of the G-code
    let head = c
        .tool("plan", json!({"path": "knob/knob.marcam.toml"}))
        .unwrap();
    let p0 = &head["setups"][0]["programs"][0];
    assert_eq!(p0["gcode_complete"], false);
    assert_eq!(p0["gcode"].as_str().unwrap().lines().count(), 40);
    // some operations only; one new operation with its defaults
    let only = c
        .tool(
            "plan",
            json!({"path": "knob/knob.marcam.toml", "operations": ["op1"]}),
        )
        .unwrap();
    assert_eq!(only["setups"][0]["programs"].as_array().unwrap().len(), 1);
    let new = c
        .tool("plan", json!({"path": "knob/knob.marcam.toml", "operation": {"type": "rotary_finish", "feed": 500.0}, "name": "finish"}))
        .unwrap();
    let np = &new["setups"][0]["programs"][0];
    assert_eq!(np["name"], "finish");
    assert_eq!(np["operations"][0]["type"], "rotary_finish");
    let d = c
        .tool(
            "operation_defaults",
            json!({"path": "knob/knob.marcam.toml", "type": "rotary_finish"}),
        )
        .unwrap();
    assert_eq!(d["type"], "rotary_finish");
    assert!(d["feed"].is_number() && d["tool"].is_number(), "{d}");
    // the stock simulated: rotary, each program's material left
    let s = c
        .tool(
            "simulate",
            json!({"path": "knob/knob.marcam.toml", "resolution": "coarse"}),
        )
        .unwrap();
    let sp = s["programs"].as_array().unwrap();
    assert_eq!(sp.len(), 2);
    assert!(sp[0]["max_left"].is_number() && sp[0].get("r").is_none());
    // an inline project (no files): planned the same
    let text = std::fs::read_to_string(dir.join("knob/knob.marcam.toml")).unwrap();
    let inline = c
        .tool("plan", json!({"project": text, "gcode": "full"}))
        .unwrap();
    assert_eq!(
        inline["setups"][0]["programs"][0]["gcode"],
        progs[0]["gcode"]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_plate_three_axis() {
    let dir = work("plate");
    let mut c = Client::start(&["--allow", dir.to_str().unwrap()], None);
    let r = c
        .tool("plan", json!({"path": "plate/plate.marcam.toml"}))
        .unwrap();
    let progs = r["setups"][0]["programs"].as_array().unwrap();
    assert_eq!(progs.len(), 2);
    assert!(progs
        .iter()
        .all(|p| p["issues"].as_array().unwrap().is_empty()));
    let pk = c
        .tool("pickables", json!({"path": "plate/plate.marcam.toml"}))
        .unwrap();
    assert!(
        !pk["pickables"].as_array().is_none_or(|a| a.is_empty()),
        "{pk}"
    );
    let s = c
        .tool(
            "simulate",
            json!({"path": "plate/plate.marcam.toml", "resolution": "coarse"}),
        )
        .unwrap();
    assert_eq!(s["kind"], "xyz");
    assert!(s["programs"][0]["max_gouge"].is_number() && s["programs"][0].get("z").is_none());
    // the dome put back as a new file: every pick found again, the project written
    std::fs::copy(dir.join("plate/dome.stl"), dir.join("plate/dome2.stl")).unwrap();
    let rm = c
        .tool("replace_model", json!({"path": "plate/plate.marcam.toml", "file": "plate/dome2.stl", "out": "plate/plate2.marcam.toml"}))
        .unwrap();
    assert!(rm["operations"].is_array(), "{rm}");
    let written = std::fs::read_to_string(dir.join("plate/plate2.marcam.toml")).unwrap();
    assert!(written.contains("dome2.stl"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn files_only_inside_the_roots() {
    let dir = work("roots");
    let other = work("other");
    // the client's roots
    let mut c = Client::start(&[], Some(vec![file_uri(&dir)]));
    assert!(c
        .tool("open_project", json!({"path": "knob/knob.marcam.toml"}))
        .is_ok());
    let abs_in = dir.join("plate/plate.marcam.toml");
    assert!(c
        .tool("open_project", json!({"path": abs_in.to_str().unwrap()}))
        .is_ok());
    let abs_out = other.join("knob/knob.marcam.toml");
    let e = c
        .tool("open_project", json!({"path": abs_out.to_str().unwrap()}))
        .unwrap_err();
    assert!(e.contains("outside"), "{e}");
    assert!(c
        .tool("open_project", json!({"path": "../x.toml"}))
        .is_err());
    // nor written outside
    let e = c
        .tool(
            "plan",
            json!({"path": "knob/knob.marcam.toml", "out": other.join("out").to_str().unwrap()}),
        )
        .unwrap_err();
    assert!(e.contains("outside"), "{e}");
    assert!(!other.join("out").exists());
    drop(c);
    // no roots, no --allow: no files at all (an inline project still plans)
    let mut none = Client::start(&[], None);
    let e = none
        .tool("open_project", json!({"path": abs_in.to_str().unwrap()}))
        .unwrap_err();
    assert!(e.contains("no folder"), "{e}");
    let text = std::fs::read_to_string(dir.join("knob/knob.marcam.toml")).unwrap();
    assert!(none.tool("plan", json!({"project": text})).is_ok());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&other);
}
