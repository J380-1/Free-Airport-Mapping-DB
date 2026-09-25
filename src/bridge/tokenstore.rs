//! Navigraph token stores of sealed aircraft.
//!
//! Marketplace/encrypted aircraft (e.g. the iniBuilds A380) cannot be patched: their EFB
//! scripts are sealed, so the token-handler rewrite never reaches them and the OANS
//! gauge keeps reporting `ARPT NAV NOT AVAILABLE (NAVIGRAPH)`. Those aircraft still
//! keep writable per-aircraft data outside the sealed package — the MSFS WASM work
//! folders — including the file the EFB persists its Navigraph token in
//! (`navigraph.txt` on the A350, and the same arrangement on the A380).
//!
//! Seeding that file with the bridge placeholder performs exactly the EFB patch by
//! other means: the gauge forwards the token as its bearer, the bridge ignores bearer
//! tokens, and the aircraft's own map draws from local data. No sealed files are
//! touched, no Navigraph account is needed, and every write keeps a backup next to
//! the file so `unseed-token` restores the original byte for byte.
//!
//! Only recognised shapes are ever written: a bare-token `navigraph*.txt`, or a JSON
//! store where a single access/id-token value is replaced and everything else is kept.
//! Anything else is reported as unrecognised and left alone. File contents are never
//! logged or included in reports — a token is a secret even when it is ours.

use super::patcher::{BACKUP_SUFFIX, A350_TOKEN};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Files larger than this are never read (token stores are small).
const MAX_STORE: u64 = 64 * 1024;
/// How deep below a package work folder to look.
const MAX_DEPTH: usize = 4;

/// A recognised token-store shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StoreKind {
    /// Bare token in a `navigraph*.txt` file.
    NavigraphTxt,
    /// JSON object with an access/id-token string value (only that value is replaced).
    TokenJson,
    /// Token-ish file in an unknown shape: reported, never written.
    Unknown,
}

/// One token store found on this machine.
#[derive(Debug, Clone)]
pub struct TokenStore {
    pub sim: String,
    pub package: String,
    pub path: PathBuf,
    pub kind: StoreKind,
    /// Already holding the bridge placeholder.
    pub seeded: bool,
    /// A backup from an earlier seeding is present.
    pub backed_up: bool,
}

/// WASM work roots that exist on this machine: (sim name, folder holding per-package data).
pub fn wasm_roots() -> Vec<(String, PathBuf)> {
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let roaming = std::env::var("APPDATA").unwrap_or_default();
    let cands = [
        ("MSFS 2024", format!("{local}/Packages/Microsoft.Limitless_8wekyb3d8bbwe/LocalState/WASM/MSFS2024")),
        ("MSFS 2024", format!("{local}/Packages/Microsoft.Limitless_8wekyb3d8bbwe/LocalState/packages")),
        ("MSFS 2024 (Steam)", format!("{roaming}/Microsoft Flight Simulator 2024/WASM/MSFS2024")),
        ("MSFS 2024 (Steam)", format!("{roaming}/Microsoft Flight Simulator 2024/Packages")),
        ("MSFS 2020", format!("{local}/Packages/Microsoft.FlightSimulator_8wekyb3d8bbwe/LocalState/packages")),
        ("MSFS 2020 (Steam)", format!("{roaming}/Microsoft Flight Simulator/Packages")),
    ];
    let mut out = Vec::new();
    for (sim, dir) in cands {
        let dir = PathBuf::from(dir);
        if dir.is_dir() && !out.iter().any(|(_, d): &(_, PathBuf)| d == &dir) {
            out.push((sim.to_string(), dir));
        }
    }
    out
}

/// iniBuilds work folders (and only those): the A380's Marketplace folder name is not
/// published, so anything iniBuilds-shaped — or A380-shaped but not FlyByWire's — counts.
fn wanted_package(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("inibuilds") || (n.contains("a380") && !n.contains("flybywire") && !n.contains("fbw"))
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, depth + 1, out);
        } else {
            out.push(p);
        }
    }
}

/// JSON key holding the token, when the store is a JSON object.
fn token_key(v: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    const KEYS: [&str; 4] = ["access_token", "id_token", "refresh_token", "token"];
    v.keys().find(|k| KEYS.contains(&k.to_ascii_lowercase().as_str())).cloned()
}

fn classify(path: &Path) -> Option<(StoreKind, bool)> {
    if fs::metadata(path).map_or(true, |m| m.len() > MAX_STORE) {
        return None;
    }
    let name = path.file_name().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if name.ends_with(BACKUP_SUFFIX) {
        return None; // our own backups are not stores
    }
    if !(name.contains("navigraph") || name.contains("token") || name.contains("auth")) {
        return None;
    }
    let Ok(text) = fs::read_to_string(path) else { return None };
    if name.ends_with(".txt") || name.contains("navigraph") && !name.contains('.') {
        return Some((StoreKind::NavigraphTxt, text.trim() == A350_TOKEN));
    }
    if name.ends_with(".json") {
        if let Ok(serde_json::Value::Object(m)) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(k) = token_key(&m) {
                let seeded = m.get(&k).and_then(|v| v.as_str()) == Some(A350_TOKEN);
                return Some((StoreKind::TokenJson, seeded));
            }
        }
        return Some((StoreKind::Unknown, false));
    }
    Some((StoreKind::Unknown, false))
}

/// Token stores under the given (sim, root) folders.
pub fn scan_roots(roots: &[(String, PathBuf)]) -> Vec<TokenStore> {
    let mut out = Vec::new();
    for (sim, root) in roots {
        let Ok(rd) = fs::read_dir(root) else { continue };
        for pkg in rd.flatten() {
            if !pkg.path().is_dir() {
                continue;
            }
            let name = pkg.file_name().to_string_lossy().to_string();
            if !wanted_package(&name) {
                continue;
            }
            let mut files = Vec::new();
            walk(&pkg.path(), 0, &mut files);
            for f in files {
                if let Some((kind, seeded)) = classify(&f) {
                    let backed_up = PathBuf::from(format!("{}{}", f.display(), BACKUP_SUFFIX)).is_file();
                    out.push(TokenStore { sim: sim.clone(), package: name.clone(), path: f, kind, seeded, backed_up });
                }
            }
        }
    }
    out.sort_by(|a, b| (&a.sim, &a.package, &a.path).cmp(&(&b.sim, &b.package, &b.path)));
    out
}

/// Token stores on this machine.
pub fn scan() -> Vec<TokenStore> {
    scan_roots(&wasm_roots())
}

fn backup(path: &Path) -> Result<PathBuf> {
    let backup = PathBuf::from(format!("{}{}", path.display(), BACKUP_SUFFIX));
    if !backup.is_file() {
        fs::copy(path, &backup).with_context(|| format!("backup {}", path.display()))?;
    }
    Ok(backup)
}

/// Write the bridge placeholder into a recognised store (backup kept).
/// Returns false when it already held the placeholder. Refuses unknown shapes.
pub fn seed(store: &TokenStore) -> Result<bool> {
    let text = fs::read_to_string(&store.path).with_context(|| format!("read {}", store.path.display()))?;
    match store.kind {
        StoreKind::NavigraphTxt => {
            if text.trim() == A350_TOKEN {
                return Ok(false);
            }
            backup(&store.path)?;
            fs::write(&store.path, A350_TOKEN).with_context(|| format!("write {}", store.path.display()))?;
            log::info!("{}: Navigraph token store seeded with the bridge placeholder (backup kept)", store.path.display());
            Ok(true)
        }
        StoreKind::TokenJson => {
            let mut v: serde_json::Value = serde_json::from_str(&text).with_context(|| format!("parse {}", store.path.display()))?;
            let Some(k) = v.as_object().and_then(token_key) else {
                anyhow::bail!("{} is no longer a recognised token store", store.path.display())
            };
            if v.get(&k).and_then(|x| x.as_str()) == Some(A350_TOKEN) {
                return Ok(false);
            }
            backup(&store.path)?;
            v[k] = serde_json::Value::from(A350_TOKEN);
            fs::write(&store.path, serde_json::to_string_pretty(&v)?).with_context(|| format!("write {}", store.path.display()))?;
            log::info!("{}: Navigraph token store seeded with the bridge placeholder (backup kept)", store.path.display());
            Ok(true)
        }
        StoreKind::Unknown => {
            anyhow::bail!("{} is not a recognised token store; leaving it alone", store.path.display())
        }
    }
}

/// Restore a seeded store from its backup. Returns false when there was no backup.
pub fn unseed(store: &TokenStore) -> Result<bool> {
    let backup = PathBuf::from(format!("{}{}", store.path.display(), BACKUP_SUFFIX));
    if !backup.is_file() {
        return Ok(false);
    }
    fs::copy(&backup, &store.path).with_context(|| format!("restore {}", store.path.display()))?;
    let _ = fs::remove_file(&backup);
    log::info!("{}: token store restored from backup", store.path.display());
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(name: &str) -> PathBuf {
        let r = std::env::temp_dir().join(format!("amdb-tokenstore-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&r);
        r
    }

    #[test]
    fn txt_store_round_trips() {
        let r = root("txt");
        let pkg = r.join("root").join("inibuilds-a380-airliner").join("work");
        fs::create_dir_all(&pkg).unwrap();
        fs::write(pkg.join("navigraph.txt"), "eyJhbGciOiJSUzI1NiJ9.old").unwrap();
        fs::write(pkg.join("notes.txt"), "nothing").unwrap();
        let other = r.join("root").join("flybywire-aircraft-a380-842").join("work");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("navigraph.txt"), "real").unwrap();
        let roots = vec![("sim".to_string(), r.join("root"))];
        let found = scan_roots(&roots);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].kind, StoreKind::NavigraphTxt);
        assert!(!found[0].seeded);
        assert!(seed(&found[0]).unwrap());
        assert_eq!(fs::read_to_string(&found[0].path).unwrap(), A350_TOKEN);
        assert!(scan_roots(&roots)[0].seeded);
        assert!(!seed(&scan_roots(&roots)[0]).unwrap(), "second seeding is a no-op");
        assert!(unseed(&found[0]).unwrap());
        assert_eq!(fs::read_to_string(&found[0].path).unwrap(), "eyJhbGciOiJSUzI1NiJ9.old");
        assert!(!unseed(&found[0]).unwrap(), "second restore is a no-op");
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn json_store_replaces_only_the_token() {
        let r = root("json");
        let pkg = r.join("root").join("inibuilds-aircraft-a350");
        fs::create_dir_all(&pkg).unwrap();
        fs::write(pkg.join("auth.json"), r#"{"access_token": "old", "pilot": "J380", "expires": 123}"#).unwrap();
        fs::write(pkg.join("auth-extra.json"), r#"{"unrelated": true}"#).unwrap();
        let roots = vec![("sim".to_string(), r.join("root"))];
        let found = scan_roots(&roots);
        assert_eq!(found.len(), 2, "{found:?}");
        let token = found.iter().find(|s| s.kind == StoreKind::TokenJson).unwrap();
        assert!(seed(token).unwrap());
        let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(&token.path).unwrap()).unwrap();
        assert_eq!(v["access_token"], A350_TOKEN);
        assert_eq!(v["pilot"], "J380");
        let unknown = found.iter().find(|s| s.kind == StoreKind::Unknown).unwrap();
        assert!(seed(unknown).is_err(), "unknown shapes are refused");
        assert!(unseed(token).unwrap());
        let _ = fs::remove_dir_all(&r);
    }
}
