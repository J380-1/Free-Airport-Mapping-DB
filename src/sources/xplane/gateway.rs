//! X-Plane Scenery Gateway client (free, no key). Downloads the recommended scenery
//! pack for an ICAO and extracts its apt.dat text.

use crate::cache::Cache;
use crate::sources::http::Http;
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use serde::Deserialize;
use std::io::Read;

const BASE: &str = "https://gateway.x-plane.com/apiv1";

#[derive(Debug, Deserialize)]
struct AirportResp {
    airport: AirportInfo,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AirportInfo {
    pub icao: String,
    pub airport_name: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub elevation: Option<f64>,
    pub recommended_scenery_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct SceneryResp {
    scenery: SceneryPack,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SceneryPack {
    scenery_id: i64,
    master_zip_blob: String,
}

/// Look up an airport on the Gateway.
pub fn airport_info(http: &Http, cache: &Cache, icao: &str) -> Result<AirportInfo> {
    let key = format!("gateway/airport/{}.json", icao.to_uppercase());
    let text = cache.get_or_fetch_text(&key, || {
        crate::term::step(Some(icao), &format!("GET {BASE}/airport/{}", icao.to_uppercase()));
        http.get_text(&format!("{BASE}/airport/{}", icao.to_uppercase()))
    })?;
    let r: AirportResp = serde_json::from_str(&text).context("gateway airport json")?;
    Ok(r.airport)
}

/// Download (or reuse cached) apt.dat text for the recommended scenery of `icao`.
/// Returns `Ok(None)` when the Gateway has no scenery for this airport.
pub fn fetch_aptdat(http: &Http, cache: &Cache, icao: &str, known_scenery: Option<Option<i64>>) -> Result<Option<String>> {
    let icao = icao.to_uppercase();
    // The Gateway list (index cache) already carries the scenery id; skip the lookup then.
    let id = match known_scenery {
        Some(Some(id)) => id,
        Some(None) => return Ok(None),
        None => match airport_info(http, cache, &icao)?.recommended_scenery_id {
            Some(id) => id,
            None => return Ok(None),
        },
    };
    let key = format!("gateway/aptdat/{icao}_{id}.dat");
    let text = cache.get_or_fetch_text(&key, || {
        let t0 = std::time::Instant::now();
        crate::term::step(Some(&icao), &format!("GET {BASE}/scenery/{id}"));
        let json = http.get_text(&format!("{BASE}/scenery/{id}"))?;
        let r: SceneryResp = serde_json::from_str(&json).context("gateway scenery json")?;
        let bytes = base64::engine::general_purpose::STANDARD.decode(r.scenery.master_zip_blob.as_bytes()).context("base64")?;
        let text = extract_aptdat(&bytes, &icao).with_context(|| format!("scenery {} zip", r.scenery.scenery_id))?;
        crate::term::step(Some(&icao), &format!("Scenery pack {} received: {} zip, apt.dat {} in {}", r.scenery.scenery_id, crate::term::human_bytes(bytes.len() as u64), crate::term::human_bytes(text.len() as u64), crate::term::human_secs(t0.elapsed().as_secs_f64())));
        Ok(text)
    })?;
    Ok(Some(text))
}

/// Find the airport data file inside a Gateway scenery zip (`<ICAO>.dat` or `apt.dat`,
/// possibly nested one level in `*_Scenery_Pack.zip`).
pub fn extract_aptdat(zip_bytes: &[u8], icao: &str) -> Result<String> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).context("open zip")?;
    let names: Vec<String> = (0..z.len()).filter_map(|i| z.by_index(i).ok().map(|f| f.name().to_string())).collect();
    let want_dat = |n: &str| {
        let l = n.to_ascii_lowercase();
        l.ends_with(&format!("{}.dat", icao.to_ascii_lowercase())) || l.ends_with("apt.dat")
    };
    if let Some(n) = names.iter().find(|n| want_dat(n)) {
        let mut b = Vec::new();
        z.by_name(n)?.read_to_end(&mut b)?;
        // Some files are Latin-1; lossy decoding keeps the numeric rows intact.
        return Ok(String::from_utf8_lossy(&b).into_owned());
    }
    // Nested pack zip.
    if let Some(n) = names.iter().find(|n| n.to_ascii_lowercase().ends_with(".zip")) {
        let mut b = Vec::new();
        z.by_name(n)?.read_to_end(&mut b)?;
        return extract_aptdat(&b, icao);
    }
    Err(anyhow!("no apt.dat in scenery zip (entries: {names:?})"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (n, b) in entries {
                w.start_file(*n, opts).unwrap();
                w.write_all(b).unwrap();
            }
            w.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn extracts_icao_dat_and_nested_zip() {
        let inner = make_zip(&[("Earth nav data/apt.dat", b"I\n1200\n1 1 0 0 KAAA X\n99\n")]);
        let outer = make_zip(&[("KAAA.txt", b"notes"), ("KAAA_Scenery_Pack.zip", &inner)]);
        let s = extract_aptdat(&outer, "KAAA").unwrap();
        assert!(s.contains("KAAA X"));
        let direct = make_zip(&[("KBBB.dat", b"I\n1200\n1 1 0 0 KBBB Y\n99\n")]);
        assert!(extract_aptdat(&direct, "KBBB").unwrap().contains("KBBB Y"));
        assert!(extract_aptdat(&make_zip(&[("readme.txt", b"x")]), "KCCC").is_err());
    }
}
