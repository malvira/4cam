//! Project files: TOML (`.marcam.toml`, what marcam writes), JSON (read forever: 0.1 jobs and
//! 0.2 projects), and the single-file bundle (`.marcam`: a zip holding `project.toml` and the
//! part files it names, what the web app saves).
//!
//! The document model is the same in every form: TOML is turned into the same JSON the engine
//! has always read, so nothing else changes. Written TOML keeps the model's key order, puts
//! operations under `[[setups.operations]]`, leaves out what is not set, and writes numbers
//! without float noise (3.175, not 3.1750000000000003).

use serde_json::Value;
use std::borrow::Cow;

/// The text of a project or job file (TOML or JSON) as JSON: JSON as it is, TOML converted.
pub fn to_json_text(text: &str) -> Result<Cow<'_, str>, String> {
    let t = text.trim_start_matches('\u{feff}').trim_start();
    if t.starts_with('{') || t.is_empty() {
        return Ok(Cow::Borrowed(text));
    }
    let v: toml::Value =
        toml::from_str(text).map_err(|e| format!("not a TOML or JSON project: {e}"))?;
    let j = toml_to_json(v);
    Ok(Cow::Owned(j.to_string()))
}

fn toml_to_json(v: toml::Value) -> Value {
    match v {
        toml::Value::String(s) => Value::String(s),
        toml::Value::Integer(i) => Value::from(i),
        toml::Value::Float(f) => serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number),
        toml::Value::Boolean(b) => Value::Bool(b),
        toml::Value::Datetime(d) => Value::String(d.to_string()),
        toml::Value::Array(a) => Value::Array(a.into_iter().map(toml_to_json).collect()),
        toml::Value::Table(t) => {
            Value::Object(t.into_iter().map(|(k, v)| (k, toml_to_json(v))).collect())
        }
    }
}

/// A float as written: rounded to a nanometre-sized step (1e-9 of its unit) when that is within
/// a part in 10^12 of it, so computed values print as typed.
fn tidy(f: f64) -> f64 {
    if !f.is_finite() || f.abs() >= 1e6 {
        return f;
    }
    let r = (f * 1e9).round() / 1e9;
    if (r - f).abs() <= 1e-12 * f.abs().max(1.0) {
        // and through the shortest decimal that names it
        format!("{r}").parse().unwrap_or(r)
    } else {
        f
    }
}

/// A JSON value as TOML: nulls left out (TOML has none), floats tidied, whole numbers that
/// were written as floats kept as floats (3.0 stays 3.0).
fn json_to_toml(v: &Value) -> Option<toml::Value> {
    Some(match v {
        Value::Null => return None,
        Value::Bool(b) => toml::Value::Boolean(*b),
        Value::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) if !n.is_f64() => toml::Value::Integer(i),
            (_, Some(f)) => toml::Value::Float(tidy(f)),
            _ => toml::Value::Integer(n.as_u64().unwrap_or(0) as i64),
        },
        Value::String(s) => toml::Value::String(s.clone()),
        Value::Array(a) => toml::Value::Array(a.iter().filter_map(json_to_toml).collect()),
        Value::Object(m) => {
            let mut t = toml::map::Map::new();
            for (k, v) in m {
                if let Some(x) = json_to_toml(v) {
                    t.insert(k.clone(), x);
                }
            }
            toml::Value::Table(t)
        }
    })
}

const HEADER: &str = "# marcam project (TOML). Lengths in mm, angles in degrees, feeds in mm/min unless a\n# key says otherwise. Comments are allowed and ignored. See docs/api.md.\n\n";

/// A document as TOML, with a header comment. `value` is the document as JSON (its key order
/// is the map's: prefer [`project_to_toml`] for a project, which keeps the model's order).
pub fn to_toml(value: &Value) -> Result<String, String> {
    let t = json_to_toml(value).ok_or("an empty document")?;
    Ok(format!(
        "{HEADER}{}",
        toml::to_string(&t).map_err(|e| e.to_string())?
    ))
}

/// A project as TOML in the model's own order (schema, name, models, tools, setups; each
/// operation's fields as its type lists them), floats tidied, nothing unset written.
pub fn project_to_toml(p: &crate::project::Project) -> Result<String, String> {
    let t = toml::Value::try_from(p).map_err(|e| e.to_string())?;
    Ok(format!(
        "{HEADER}{}",
        toml::to_string(&tidy_toml(t)).map_err(|e| e.to_string())?
    ))
}

/// Floats tidied (see `tidy`), all through.
fn tidy_toml(v: toml::Value) -> toml::Value {
    match v {
        toml::Value::Float(f) => toml::Value::Float(tidy(f)),
        toml::Value::Array(a) => toml::Value::Array(a.into_iter().map(tidy_toml).collect()),
        toml::Value::Table(t) => {
            toml::Value::Table(t.into_iter().map(|(k, v)| (k, tidy_toml(v))).collect())
        }
        other => other,
    }
}

/// A project read from any of its forms (TOML, JSON, 0.1 job) and written as TOML.
pub fn convert_to_toml(text: &str) -> Result<String, String> {
    let (p, _) = crate::project::Project::from_json_any(&to_json_text(text)?)?;
    project_to_toml(&p)
}

// ------------------------------------------------------------------ the bundle (.marcam)

/// The name of the project inside a bundle.
pub const BUNDLE_PROJECT: &str = "project.toml";

/// A single-file project: a zip of `project.toml` and the files it names (meshes, images),
/// stored at the paths the project uses. Readable by any zip tool.
pub fn write_bundle(project_toml: &str, files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut entries: Vec<(&str, &[u8])> = vec![(BUNDLE_PROJECT, project_toml.as_bytes())];
    for (name, bytes) in files {
        entries.push((name.as_str(), bytes.as_slice()));
    }
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in &entries {
        // deflated when it saves room (text, meshes); stored otherwise
        let packed = deflate(data);
        let (method, body): (u16, &[u8]) = if packed.len() < data.len() {
            (8, &packed)
        } else {
            (0, data)
        };
        let mut crc = flate2::Crc::new();
        crc.update(data);
        let crc = crc.sum();
        let offset = out.len() as u32;
        let n = name.as_bytes();
        // local header
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 names
        out.extend_from_slice(&method.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // time
        out.extend_from_slice(&0x21u16.to_le_bytes()); // date (1980-01-01)
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(n.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(n);
        out.extend_from_slice(body);
        // its central directory record
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // made by
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&0x0800u16.to_le_bytes());
        central.extend_from_slice(&method.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0x21u16.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(body.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(n.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0; 8]); // extra, comment, disk, internal attributes
        central.extend_from_slice(&0u32.to_le_bytes()); // external attributes
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(n);
    }
    let cd_offset = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

fn deflate(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = e.write_all(data);
    e.finish().unwrap_or_default()
}

/// Whether `bytes` is a zip (a bundle).
pub fn is_bundle(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[..4] == [0x50, 0x4b, 0x03, 0x04]
}

/// A bundle's project (TOML or JSON text) and its other files, by their paths.
#[allow(clippy::type_complexity)]
pub fn read_bundle(bytes: &[u8]) -> Result<(String, Vec<(String, Vec<u8>)>), String> {
    use std::io::Read;
    let u16_at = |i: usize| -> Result<u16, String> {
        bytes
            .get(i..i + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .ok_or("a truncated bundle".to_string())
    };
    let u32_at = |i: usize| -> Result<u32, String> {
        bytes
            .get(i..i + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .ok_or("a truncated bundle".to_string())
    };
    // the end of central directory record (no zip comment, or a short one)
    let eocd = (0..bytes.len().saturating_sub(21))
        .rev()
        .take(65_557)
        .find(|&i| bytes[i..].starts_with(&[0x50, 0x4b, 0x05, 0x06]))
        .ok_or("not a marcam bundle (no zip directory)")?;
    let count = u16_at(eocd + 10)? as usize;
    let mut at = u32_at(eocd + 16)? as usize;
    let mut project = None;
    let mut files = Vec::new();
    for _ in 0..count {
        if u32_at(at)? != 0x0201_4b50 {
            return Err("a damaged bundle (its directory)".into());
        }
        let method = u16_at(at + 10)?;
        let csize = u32_at(at + 20)? as usize;
        let size = u32_at(at + 24)? as usize;
        let nlen = u16_at(at + 28)? as usize;
        let xlen = u16_at(at + 30)? as usize;
        let clen = u16_at(at + 32)? as usize;
        let local = u32_at(at + 42)? as usize;
        let name = String::from_utf8_lossy(
            bytes
                .get(at + 46..at + 46 + nlen)
                .ok_or("a truncated bundle")?,
        )
        .into_owned();
        at += 46 + nlen + xlen + clen;
        let lstart = local + 30 + u16_at(local + 26)? as usize + u16_at(local + 28)? as usize;
        let body = bytes
            .get(lstart..lstart + csize)
            .ok_or("a truncated bundle")?;
        let data = match method {
            0 => body.to_vec(),
            8 => {
                let mut out = Vec::with_capacity(size);
                flate2::read::DeflateDecoder::new(body)
                    .read_to_end(&mut out)
                    .map_err(|e| format!("{name}: {e}"))?;
                out
            }
            m => {
                return Err(format!(
                "{name}: compressed with method {m}, which marcam does not read (store or deflate)"
            ))
            }
        };
        if name.ends_with('/') {
            continue;
        }
        if name == BUNDLE_PROJECT
            || (project.is_none()
                && (name.ends_with(".marcam.toml") || name.ends_with(".project.json")))
        {
            project = Some(String::from_utf8(data).map_err(|_| format!("{name}: not text"))?);
        } else {
            files.push((name, data));
        }
    }
    Ok((
        project.ok_or(format!("the bundle has no {BUNDLE_PROJECT}"))?,
        files,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_without_noise() {
        assert_eq!(tidy(3.1750000000000003), 3.175);
        assert_eq!(tidy(0.1 + 0.2), 0.3);
        assert_eq!(
            tidy(1.0 / 3.0),
            1.0 / 3.0,
            "a value that is not a short decimal stays"
        );
        assert_eq!(tidy(-0.0), 0.0);
    }

    #[test]
    fn bundles_round_trip() {
        let files = vec![
            ("knob.stl".to_string(), vec![7u8; 5000]),
            ("img/a.png".to_string(), vec![1, 2, 3]),
        ];
        let z = write_bundle("name = \"x\"\n", &files);
        assert!(is_bundle(&z));
        let (p, f) = read_bundle(&z).unwrap();
        assert_eq!(p, "name = \"x\"\n");
        assert_eq!(f, files);
    }
}
