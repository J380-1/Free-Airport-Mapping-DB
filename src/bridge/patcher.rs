//! Points installed aircraft at the local bridge by rewriting the Navigraph AMDB
//! host inside their JavaScript bundles. Backups are kept next to each file and a
//! record is written so `unpatch` restores them exactly.
//!
//! Handled forms:
//! * literal `https://amdb.api.navigraph.com` (FlyByWire builds and anything on the fbw-sdk);
//! * Navigraph SDK templates `https://amdb.api.${fn()}` (the host is computed), which
//!   become `http://127.0.0.1:PORT/${fn()}` — the server ignores the extra prefix.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PatchRecord {
    pub files: Vec<PatchedFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchedFile {
    pub path: PathBuf,
    pub backup: PathBuf,
    pub replacements: usize,
}

pub const BACKUP_SUFFIX: &str = ".amdb-bridge.bak";

/// Candidate Community folders for MSFS 2020 and 2024 (Store and Steam) on this machine.
pub fn detect_community_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let roaming = std::env::var("APPDATA").unwrap_or_default();
    let cfgs = [
        format!("{local}/Packages/Microsoft.FlightSimulator_8wekyb3d8bbwe/LocalCache/UserCfg.opt"),
        format!("{local}/Packages/Microsoft.Limitless_8wekyb3d8bbwe/LocalCache/UserCfg.opt"),
        format!("{roaming}/Microsoft Flight Simulator/UserCfg.opt"),
        format!("{roaming}/Microsoft Flight Simulator 2024/UserCfg.opt"),
    ];
    for c in cfgs {
        if let Ok(t) = fs::read_to_string(&c) {
            for line in t.lines() {
                let l = line.trim();
                if let Some(rest) = l.strip_prefix("InstalledPackagesPath") {
                    let p = rest.trim().trim_matches('"');
                    let d = Path::new(p).join("Community");
                    if d.is_dir() && !out.contains(&d) {
                        out.push(d);
                    }
                }
            }
        }
    }
    for d in [
        format!("{local}/Packages/Microsoft.FlightSimulator_8wekyb3d8bbwe/LocalCache/Packages/Community"),
        format!("{local}/Packages/Microsoft.Limitless_8wekyb3d8bbwe/LocalCache/Packages/Community"),
    ] {
        let d = PathBuf::from(d);
        if d.is_dir() && !out.contains(&d) {
            out.push(d);
        }
    }
    out
}

fn walk_js(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk_js(&p, out);
        } else if p.extension().and_then(|s| s.to_str()).map_or(false, |x| x.eq_ignore_ascii_case("js") || x.eq_ignore_ascii_case("mjs")) {
            out.push(p);
        }
    }
}

/// A file that references the Navigraph AMDB host, with the package it belongs to.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub package: String,
    pub path: PathBuf,
    pub literal_hits: usize,
    pub template_hits: usize,
}

pub fn scan(community: &Path) -> Vec<Candidate> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(community) else { return out };
    for pkg in rd.flatten() {
        let pdir = pkg.path();
        if !pdir.is_dir() {
            continue;
        }
        let mut js = Vec::new();
        walk_js(&pdir.join("html_ui"), &mut js);
        for f in js {
            let Ok(text) = fs::read_to_string(&f) else { continue };
            let literal = text.matches(super::NAVIGRAPH_AMDB_HOST).count();
            let template = text.matches("https://amdb.api.${").count();
            if literal + template > 0 {
                out.push(Candidate { package: pkg.file_name().to_string_lossy().to_string(), path: f, literal_hits: literal, template_hits: template });
            }
        }
    }
    out
}

fn record_path(community: &Path) -> PathBuf {
    community.join(".amdb-bridge-patches.json")
}

pub fn load_record(community: &Path) -> PatchRecord {
    fs::read_to_string(record_path(community)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save_record(community: &Path, r: &PatchRecord) -> Result<()> {
    fs::write(record_path(community), serde_json::to_string_pretty(r)?)?;
    Ok(())
}

/// Patch every candidate in a Community folder. Returns the files changed.
pub fn patch(community: &Path, port: u16, dry_run: bool) -> Result<Vec<PatchedFile>> {
    let local = format!("http://127.0.0.1:{port}");
    let mut record = load_record(community);
    let mut done = Vec::new();
    for c in scan(community) {
        let text = fs::read_to_string(&c.path)?;
        let patched = text.replace(super::NAVIGRAPH_AMDB_HOST, &local).replace("https://amdb.api.${", &format!("{local}/${{"));
        if patched == text {
            continue;
        }
        let n = c.literal_hits + c.template_hits;
        log::info!("{}{}: {} replacement(s) in {}", if dry_run { "[dry-run] " } else { "" }, c.package, n, c.path.display());
        if dry_run {
            done.push(PatchedFile { path: c.path.clone(), backup: PathBuf::new(), replacements: n });
            continue;
        }
        let backup = PathBuf::from(format!("{}{}", c.path.display(), BACKUP_SUFFIX));
        if !backup.exists() {
            fs::copy(&c.path, &backup).with_context(|| format!("backup {}", c.path.display()))?;
        }
        fs::write(&c.path, patched)?;
        let pf = PatchedFile { path: c.path.clone(), backup, replacements: n };
        record.files.retain(|f| f.path != pf.path);
        record.files.push(pf.clone());
        done.push(pf);
    }
    if !dry_run && !done.is_empty() {
        save_record(community, &record)?;
    }
    Ok(done)
}

/// Restore every backed-up file in a Community folder.
pub fn unpatch(community: &Path) -> Result<usize> {
    let record = load_record(community);
    let mut n = 0;
    for f in &record.files {
        if f.backup.is_file() {
            fs::copy(&f.backup, &f.path).with_context(|| format!("restore {}", f.path.display()))?;
            let _ = fs::remove_file(&f.backup);
            n += 1;
        }
    }
    let _ = fs::remove_file(record_path(community));
    Ok(n)
}

/// Register the bridge in the simulator's exe.xml so it starts with the sim.
pub fn install_autostart(exe: &Path, args: &str) -> Result<Vec<PathBuf>> {
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let roaming = std::env::var("APPDATA").unwrap_or_default();
    let candidates = [
        format!("{local}/Packages/Microsoft.FlightSimulator_8wekyb3d8bbwe/LocalCache/exe.xml"),
        format!("{local}/Packages/Microsoft.Limitless_8wekyb3d8bbwe/LocalCache/exe.xml"),
        format!("{roaming}/Microsoft Flight Simulator/exe.xml"),
        format!("{roaming}/Microsoft Flight Simulator 2024/exe.xml"),
    ];
    let entry = format!(
        "  <Launch.Addon>\n    <Name>AMDB Bridge</Name>\n    <Disabled>False</Disabled>\n    <ManualLoad>False</ManualLoad>\n    <Path>{}</Path>\n    <CommandLine>{}</CommandLine>\n  </Launch.Addon>\n",
        exe.display(),
        args
    );
    let mut written = Vec::new();
    for c in candidates {
        let p = PathBuf::from(&c);
        if !p.is_file() {
            continue;
        }
        let text = fs::read_to_string(&p)?;
        if text.contains("<Name>AMDB Bridge</Name>") {
            continue;
        }
        let Some(pos) = text.rfind("</SimBase.Document>") else { continue };
        let new = format!("{}{}{}", &text[..pos], entry, &text[pos..]);
        fs::copy(&p, format!("{c}{BACKUP_SUFFIX}"))?;
        fs::write(&p, new)?;
        written.push(p);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patches_and_restores() {
        let dir = std::env::temp_dir().join(format!("amdb-bridge-patch-{}", std::process::id()));
        let pkg = dir.join("some-aircraft").join("html_ui").join("Pages");
        fs::create_dir_all(&pkg).unwrap();
        let f = pkg.join("bundle.js");
        let original = "fetch(`https://amdb.api.navigraph.com/v1/${icao}`); x=`https://amdb.api.${dom()}/v1/search`;";
        fs::write(&f, original).unwrap();
        let done = patch(&dir, 8770, false).unwrap();
        assert_eq!(done.len(), 1);
        let t = fs::read_to_string(&f).unwrap();
        assert!(t.contains("http://127.0.0.1:8770/v1/${icao}"));
        assert!(t.contains("http://127.0.0.1:8770/${dom()}/v1/search"));
        assert!(!t.contains("navigraph.com"));
        assert!(scan(&dir).is_empty());
        assert_eq!(unpatch(&dir).unwrap(), 1);
        assert_eq!(fs::read_to_string(&f).unwrap(), original);
        let _ = fs::remove_dir_all(&dir);
    }
}
