//! Minimal zip reader: stored, deflate and zstd entries. Enough for 3MF files, Fusion 360 tool
//! libraries (`.tools`) and Fusion design archives (`.f3d`, which use zstd). No zip64.

use std::io::Read;

fn le16(d: &[u8], o: usize) -> Result<usize, String> {
    d.get(o..o + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]) as usize)
        .ok_or_else(|| "truncated zip".to_string())
}

fn le32(d: &[u8], o: usize) -> Result<usize, String> {
    d.get(o..o + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
        .ok_or_else(|| "truncated zip".to_string())
}

/// True if `d` starts like a zip archive.
pub fn is_zip(d: &[u8]) -> bool {
    d.starts_with(b"PK\x03\x04")
}

/// Central directory: `(name, offset of its central record)` per entry.
pub fn entries(d: &[u8]) -> Result<Vec<(String, usize)>, String> {
    // end of central directory: last "PK\x05\x06" within the final 64 KiB + 22 bytes
    let lo = d.len().saturating_sub(65_557);
    let eocd = (lo..d.len().saturating_sub(21))
        .rev()
        .find(|&i| d[i..i + 4] == *b"PK\x05\x06")
        .ok_or("not a zip archive (no end of central directory)")?;
    let count = le16(d, eocd + 10)?;
    let mut at = le32(d, eocd + 16)?;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if d.get(at..at + 4) != Some(b"PK\x01\x02".as_slice()) {
            return Err("corrupt zip central directory".into());
        }
        let (n, e, c) = (le16(d, at + 28)?, le16(d, at + 30)?, le16(d, at + 32)?);
        let name = d.get(at + 46..at + 46 + n).ok_or("truncated zip")?;
        out.push((String::from_utf8_lossy(name).into_owned(), at));
        at += 46 + n + e + c;
    }
    Ok(out)
}

/// Contents of one entry.
pub fn read(d: &[u8], name: &str, entries: &[(String, usize)]) -> Result<Vec<u8>, String> {
    let &(_, cen) = entries
        .iter()
        .find(|(n, _)| n == name)
        .ok_or_else(|| format!("zip has no {name}"))?;
    let method = le16(d, cen + 10)?;
    let csize = le32(d, cen + 20)?;
    let size = le32(d, cen + 24)?;
    let local = le32(d, cen + 42)?;
    if d.get(local..local + 4) != Some(b"PK\x03\x04".as_slice()) {
        return Err("corrupt zip local header".into());
    }
    let start = local + 30 + le16(d, local + 26)? + le16(d, local + 28)?;
    let raw = d.get(start..start + csize).ok_or("truncated zip entry")?;
    let mut out = Vec::with_capacity(size);
    match method {
        0 => out.extend_from_slice(raw),
        8 => {
            flate2::read::DeflateDecoder::new(raw)
                .read_to_end(&mut out)
                .map_err(|e| format!("zip {name}: {e}"))?;
        }
        93 => {
            ruzstd::decoding::StreamingDecoder::new(raw)
                .map_err(|e| format!("zip {name}: {e}"))?
                .read_to_end(&mut out)
                .map_err(|e| format!("zip {name}: {e}"))?;
        }
        m => return Err(format!("zip {name}: unsupported compression method {m}")),
    }
    Ok(out)
}

/// Read the first entry whose name satisfies `pick`.
pub fn read_where(d: &[u8], pick: impl Fn(&str) -> bool) -> Result<(String, Vec<u8>), String> {
    let es = entries(d)?;
    let name = es
        .iter()
        .map(|(n, _)| n.clone())
        .find(|n| pick(n))
        .ok_or("no matching entry in the archive")?;
    let data = read(d, &name, &es)?;
    Ok((name, data))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Write;

    /// Minimal zip writer for tests (stored or deflated entries, CRC left 0).
    pub(crate) fn make_zip(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data, deflate) in files {
            let body = if *deflate {
                let mut e =
                    flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(data).unwrap();
                e.finish().unwrap()
            } else {
                data.to_vec()
            };
            let method: u16 = if *deflate { 8 } else { 0 };
            let offset = out.len() as u32;
            let mut h = b"PK".to_vec();
            h.extend_from_slice(&[20, 0, 0, 0]);
            h.extend_from_slice(&method.to_le_bytes());
            h.extend_from_slice(&[0; 8]); // time, date, crc
            h.extend_from_slice(&(body.len() as u32).to_le_bytes());
            h.extend_from_slice(&(data.len() as u32).to_le_bytes());
            h.extend_from_slice(&(name.len() as u16).to_le_bytes());
            h.extend_from_slice(&[0, 0]);
            h.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&h);
            out.extend_from_slice(&body);
            let mut c = b"PK".to_vec();
            c.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
            c.extend_from_slice(&method.to_le_bytes());
            c.extend_from_slice(&[0; 8]);
            c.extend_from_slice(&(body.len() as u32).to_le_bytes());
            c.extend_from_slice(&(data.len() as u32).to_le_bytes());
            c.extend_from_slice(&(name.len() as u16).to_le_bytes());
            c.extend_from_slice(&[0; 12]);
            c.extend_from_slice(&offset.to_le_bytes());
            c.extend_from_slice(name.as_bytes());
            central.extend_from_slice(&c);
        }
        let cd_off = out.len() as u32;
        out.extend_from_slice(&central);
        let mut e = b"PK".to_vec();
        e.extend_from_slice(&[0; 4]);
        e.extend_from_slice(&(files.len() as u16).to_le_bytes());
        e.extend_from_slice(&(files.len() as u16).to_le_bytes());
        e.extend_from_slice(&(central.len() as u32).to_le_bytes());
        e.extend_from_slice(&cd_off.to_le_bytes());
        e.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&e);
        out
    }

    #[test]
    fn stored_deflated_and_missing() {
        let z = make_zip(&[
            ("a.txt", b"hello".as_slice(), false),
            ("b.json", b"{}".as_slice(), true),
        ]);
        let es = super::entries(&z).unwrap();
        assert_eq!(super::read(&z, "a.txt", &es).unwrap(), b"hello");
        assert_eq!(super::read(&z, "b.json", &es).unwrap(), b"{}");
        assert!(super::read(&z, "c", &es).is_err());
        assert!(super::entries(b"PK nope").is_err());
    }
}
