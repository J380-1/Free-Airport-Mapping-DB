//! Read apt.dat from a local X-Plane installation or an explicit file.
//!
//! Search order inside an X-Plane root: `Custom Scenery/*/Earth nav data/apt.dat`
//! (scenery_packs.ini order is not honoured; the first file containing the ICAO wins),
//! then `Global Scenery/Global Airports/Earth nav data/apt.dat`, then
//! `Resources/default scenery/default apt dat/Earth nav data/apt.dat`.

use anyhow::{Context, Result};
use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Candidate apt.dat files under an X-Plane root, most specific first.
pub fn candidate_files(xplane_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir(xplane_root.join("Custom Scenery")) {
        let mut packs: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir()).collect();
        packs.sort();
        for p in packs {
            let f = p.join("Earth nav data").join("apt.dat");
            if f.is_file() {
                out.push(f);
            }
        }
    }
    for rel in ["Global Scenery/Global Airports/Earth nav data/apt.dat", "Resources/default scenery/default apt dat/Earth nav data/apt.dat"] {
        let f = xplane_root.join(rel);
        if f.is_file() {
            out.push(f);
        }
    }
    out
}

/// Extract the text block for one airport from a (possibly huge) apt.dat file.
/// Returns `Ok(None)` if the ICAO is not in the file.
pub fn extract_airport_block(file: &Path, icao: &str) -> Result<Option<String>> {
    let f = fs::File::open(file).with_context(|| format!("open {}", file.display()))?;
    let mut r = BufReader::with_capacity(1 << 20, f);
    let icao_up = icao.to_uppercase();
    let mut line = Vec::new();
    let mut block: Option<String> = None;
    loop {
        line.clear();
        let n = r.read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        let s = String::from_utf8_lossy(&line);
        let t = s.trim_start();
        let is_header = t.starts_with("1 ") || t.starts_with("16 ") || t.starts_with("17 ");
        if is_header {
            if block.is_some() {
                break; // next airport reached
            }
            let code = t.split_whitespace().nth(4).unwrap_or("");
            if code.eq_ignore_ascii_case(&icao_up) {
                block = Some(String::from("I\n1200\n"));
            }
        }
        if let Some(b) = block.as_mut() {
            if t.starts_with("99") && t.trim().len() <= 3 {
                break;
            }
            b.push_str(&s);
        }
    }
    Ok(block.map(|mut b| {
        b.push_str("99\n");
        b
    }))
}

/// Find and extract `icao` from any apt.dat under an X-Plane root.
pub fn find_in_root(xplane_root: &Path, icao: &str) -> Result<Option<(PathBuf, String)>> {
    for f in candidate_files(xplane_root) {
        if let Some(block) = extract_airport_block(&f, icao)? {
            return Ok(Some((f, block)));
        }
    }
    Ok(None)
}

/// Cheap check whether a file looks like an apt.dat (header line "A" or "I" + version).
pub fn looks_like_aptdat(file: &Path) -> bool {
    let Ok(mut f) = fs::File::open(file) else { return false };
    let mut buf = [0u8; 64];
    let n = f.read(&mut buf).unwrap_or(0);
    let _ = f.seek(SeekFrom::Start(0));
    let head = String::from_utf8_lossy(&buf[..n]);
    let mut lines = head.lines();
    matches!(lines.next().map(str::trim), Some("I") | Some("A")) && lines.next().map_or(false, |l| l.trim_start().starts_with(|c: char| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn extracts_block_for_requested_icao() {
        let dir = std::env::temp_dir().join(format!("amdbgen-local-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("apt.dat");
        let mut f = fs::File::create(&p).unwrap();
        writeln!(f, "I\n1200 Version\n\n1 10 0 0 KAAA First\n100 30 1 0 0.25 0 0 0 09 1 2 0 0 0 0 0 0 27 1 2.1 0 0 0 0 0 0\n\n1 20 0 0 KBBB Second\n1302 city X\n99").unwrap();
        let b = extract_airport_block(&p, "kbbb").unwrap().unwrap();
        assert!(b.contains("KBBB Second"));
        assert!(b.contains("1302 city X"));
        assert!(!b.contains("KAAA"));
        assert!(b.trim_end().ends_with("99"));
        assert!(extract_airport_block(&p, "ZZZZ").unwrap().is_none());
        assert!(looks_like_aptdat(&p));
        let _ = fs::remove_dir_all(&dir);
    }
}
