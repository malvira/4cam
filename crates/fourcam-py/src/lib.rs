//! Python bindings: `import fourcam`. Same JSON API as the WebAssembly build; results come
//! back as Python dicts and lists. Errors raise `ValueError`.

use fourcam_wasm::api;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyString;

/// JSON text of a job given as `str` (JSON) or any JSON-serialisable object (e.g. a dict).
fn json_text(py: Python<'_>, v: &Bound<'_, PyAny>) -> PyResult<String> {
    if let Ok(s) = v.cast::<PyString>() {
        return Ok(s.to_string());
    }
    let json = py.import("json")?;
    json.call_method1("dumps", (v,))?.extract()
}

/// Mesh bytes from `bytes` or a file path.
fn mesh_bytes(mesh: Option<&Bound<'_, PyAny>>) -> PyResult<Vec<u8>> {
    let Some(m) = mesh else { return Ok(Vec::new()) };
    if m.is_none() {
        return Ok(Vec::new());
    }
    if let Ok(path) = m.extract::<std::path::PathBuf>() {
        if !m.is_instance_of::<pyo3::types::PyBytes>() {
            return std::fs::read(&path)
                .map_err(|e| PyValueError::new_err(format!("{}: {e}", path.display())));
        }
    }
    m.extract::<Vec<u8>>()
}

fn to_py(py: Python<'_>, v: serde_json::Value) -> PyResult<Py<PyAny>> {
    if let Some(e) = v.get("error").and_then(|e| e.as_str()) {
        return Err(PyValueError::new_err(e.to_owned()));
    }
    let json = py.import("json")?;
    Ok(json.call_method1("loads", (v.to_string(),))?.unbind())
}

/// Library version information.
#[pyfunction]
fn version(py: Python<'_>) -> PyResult<Py<PyAny>> {
    to_py(py, api::version())
}

/// The bundled knurled-knob example job (dict).
#[pyfunction]
fn example_job(py: Python<'_>) -> PyResult<Py<PyAny>> {
    to_py(
        py,
        serde_json::from_str(api::EXAMPLE_JOB).unwrap_or_default(),
    )
}

/// A built-in machine profile ("makera-z1" or "linuxcnc").
#[pyfunction]
#[pyo3(signature = (name = "makera-z1"))]
fn machine(py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
    to_py(py, api::machine(name))
}

/// Plan, post, verify and gouge-check a job.
///
/// `job`: dict or JSON text. `machine`: built-in name, dict or JSON text (default Makera Z1).
/// `mesh`: STL/3MF bytes or a file path, for jobs with `part.mesh`. Returns a dict with one
/// entry per program: `name`, `gcode`, `lines`, `issues`, `gouge`, `toolpath`.
#[pyfunction]
#[pyo3(signature = (job, machine = None, mesh = None, max_points = 200_000))]
fn plan_and_post(
    py: Python<'_>,
    job: &Bound<'_, PyAny>,
    machine: Option<&Bound<'_, PyAny>>,
    mesh: Option<&Bound<'_, PyAny>>,
    max_points: u64,
) -> PyResult<Py<PyAny>> {
    let job = json_text(py, job)?;
    let machine = match machine {
        Some(m) if !m.is_none() => json_text(py, m)?,
        _ => String::new(),
    };
    let mesh = mesh_bytes(mesh)?;
    let opts = format!(r#"{{"max_points": {max_points}}}"#);
    let v = py.detach(|| api::plan_and_post(&job, &machine, &mesh, &opts));
    to_py(py, v)
}

/// Simulate material removal of every program in order on a radial heightmap.
#[pyfunction]
#[pyo3(signature = (job, dx = 0.1, na = 360, mesh = None))]
fn simulate(
    py: Python<'_>,
    job: &Bound<'_, PyAny>,
    dx: f64,
    na: u32,
    mesh: Option<&Bound<'_, PyAny>>,
) -> PyResult<Py<PyAny>> {
    let job = json_text(py, job)?;
    let mesh = mesh_bytes(mesh)?;
    let opts = format!(r#"{{"dx": {dx}, "na": {na}}}"#);
    let v = py.detach(|| api::simulate(&job, &opts, &mesh));
    to_py(py, v)
}

#[pymodule]
fn fourcam(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(version, m)?)?;
    m.add_function(wrap_pyfunction!(example_job, m)?)?;
    m.add_function(wrap_pyfunction!(machine, m)?)?;
    m.add_function(wrap_pyfunction!(plan_and_post, m)?)?;
    m.add_function(wrap_pyfunction!(simulate, m)?)?;
    Ok(())
}
