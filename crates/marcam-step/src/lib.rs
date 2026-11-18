//! STEP (ISO 10303-21: AP203, AP214, AP242) import for marcam, with SolidCraft's kernel
//! (`solidcraft-kernel`, MIT OR Apache-2.0, on SolidCore, Apache-2.0; see docs/THIRD-PARTY.md).
//!
//! A STEP file becomes a [`marcam::Mesh`] in millimetres whose triangles carry their B-rep
//! face (`Mesh::faces`, numbered from 1 across every body in the file's order, the same for the
//! same file), so the engine's face regions are the file's faces. Every body (solids and
//! surface models, placed by the file's assemblies) is meshed within a chord tolerance.
//!
//! - [`import`]: the mesh and what came in (bodies, faces, warnings);
//! - [`to_stl`]: the same as binary STL with the faces in the facets' attribute words (what the
//!   web app and the bindings hand to the engine);
//! - [`register`]: makes `marcam::Mesh::from_bytes` (and so every project and job) read STEP.

use marcam::Mesh;

/// The default chord tolerance (mm): the farthest a facet stands off the surface.
pub const DEFAULT_CHORD: f64 = 0.01;

/// A STEP file read and meshed.
#[derive(Debug, Clone)]
pub struct StepPart {
    pub mesh: Mesh,
    /// Bodies read (solids and surface models).
    pub bodies: usize,
    /// B-rep faces (numbered 1..=faces in `mesh.faces`).
    pub faces: usize,
    /// Each body's name, in the file's order.
    pub body_names: Vec<String>,
    /// What could not be read, or was read only in part.
    pub warnings: Vec<String>,
}

/// Read a STEP file and mesh every body within `chord` mm.
pub fn import(bytes: &[u8], chord: f64) -> Result<StepPart, String> {
    if !marcam::mesh::is_step(bytes) {
        return Err("not a STEP file (no ISO-10303-21 header)".into());
    }
    let text = String::from_utf8_lossy(bytes);
    let imp = solidcraft_kernel::step_import(&text).map_err(|e| format!("STEP: {e}"))?;
    if imp.bodies.is_empty() {
        let why = imp
            .warnings
            .first()
            .cloned()
            .unwrap_or_else(|| "it has no solids or surfaces".into());
        return Err(format!("STEP: nothing to machine in the file ({why})"));
    }
    let chord = if chord.is_finite() && chord > 0.0 {
        chord
    } else {
        DEFAULT_CHORD
    };
    let tol = solidcore_tess::MeshTolerance::chord(chord);
    let mut mesh = Mesh::default();
    let mut warnings = imp.warnings.clone();
    let mut names = Vec::new();
    let mut next_face = 0u32;
    for b in &imp.bodies {
        let m = b
            .body
            .mesh(&tol)
            .map_err(|e| format!("STEP: body {:?}: {e}", b.name))?;
        if let Some(why) = &b.refused {
            warnings.push(format!("{}: read as surfaces ({why})", b.name));
        }
        let base = next_face;
        for (t, f) in m.triangles.iter().zip(&m.tri_face) {
            let p = |i: u32| {
                let v = m.positions[i as usize];
                [v.x, v.y, v.z]
            };
            mesh.triangles.push([p(t[0]), p(t[1]), p(t[2])]);
            mesh.faces.push(base + f + 1);
        }
        next_face = base + m.tri_face.iter().copied().max().map_or(0, |f| f + 1);
        names.push(b.name.clone());
    }
    Ok(StepPart {
        bodies: imp.bodies.len(),
        faces: next_face as usize,
        body_names: names,
        warnings,
        mesh,
    })
}

/// [`import`] as binary STL, each facet's attribute word its face (see
/// `marcam::mesh::FACES_HEADER`).
pub fn to_stl(bytes: &[u8], chord: f64) -> Result<Vec<u8>, String> {
    let part = import(bytes, chord)?;
    if part.faces > u16::MAX as usize {
        return Err(format!("STEP: {} faces, more than 65535", part.faces));
    }
    Ok(part.mesh.to_stl())
}

fn read_for_engine(bytes: &[u8]) -> Result<Mesh, String> {
    import(bytes, DEFAULT_CHORD).map(|p| p.mesh)
}

/// Let `marcam::Mesh::from_bytes` read STEP files (meshed at [`DEFAULT_CHORD`]).
pub fn register() {
    marcam::mesh::set_step_reader(read_for_engine);
}

// ------------------------------------------------------------------ WebAssembly
// The web app's STEP module: `ms_alloc` a buffer, write the file into it, `ms_import` it;
// the result is JSON at `ms_result_ptr` / `ms_result_len` and the STL at `ms_stl_ptr` /
// `ms_stl_len` (until the next call).
#[cfg(target_arch = "wasm32")]
mod wasm {
    use std::cell::RefCell;

    thread_local! {
        static RESULT: RefCell<(Vec<u8>, Vec<u8>)> = const { RefCell::new((Vec::new(), Vec::new())) };
    }

    #[no_mangle]
    pub extern "C" fn ms_alloc(len: usize) -> *mut u8 {
        let mut v = vec![0u8; len];
        let p = v.as_mut_ptr();
        std::mem::forget(v);
        p
    }

    /// # Safety
    /// `p` and `len` from one [`ms_alloc`].
    #[no_mangle]
    pub unsafe extern "C" fn ms_free(p: *mut u8, len: usize) {
        // SAFETY: allocated by ms_alloc with this length.
        drop(unsafe { Vec::from_raw_parts(p, len, len) });
    }

    /// # Safety
    /// `p` and `len` from [`ms_alloc`], holding the file.
    #[no_mangle]
    pub unsafe extern "C" fn ms_import(p: *const u8, len: usize, chord: f64) -> u32 {
        // SAFETY: the caller wrote `len` bytes there.
        let bytes = unsafe { std::slice::from_raw_parts(p, len) };
        let (json, stl) = match super::import(bytes, chord) {
            Ok(part) if part.faces <= u16::MAX as usize => {
                let stl = part.mesh.to_stl();
                let j = serde_json::json!({
                    "bodies": part.bodies, "faces": part.faces,
                    "triangles": part.mesh.triangles.len(),
                    "names": part.body_names, "warnings": part.warnings
                });
                (j, stl)
            }
            Ok(part) => (
                serde_json::json!({"error": format!("STEP: {} faces, more than 65535", part.faces)}),
                Vec::new(),
            ),
            Err(e) => (serde_json::json!({ "error": e }), Vec::new()),
        };
        let ok = json.get("error").is_none();
        RESULT.with(|r| *r.borrow_mut() = (json.to_string().into_bytes(), stl));
        ok as u32
    }

    #[no_mangle]
    pub extern "C" fn ms_result_ptr() -> *const u8 {
        RESULT.with(|r| r.borrow().0.as_ptr())
    }
    #[no_mangle]
    pub extern "C" fn ms_result_len() -> usize {
        RESULT.with(|r| r.borrow().0.len())
    }
    #[no_mangle]
    pub extern "C" fn ms_stl_ptr() -> *const u8 {
        RESULT.with(|r| r.borrow().1.as_ptr())
    }
    #[no_mangle]
    pub extern "C" fn ms_stl_len() -> usize {
        RESULT.with(|r| r.borrow().1.len())
    }
}
