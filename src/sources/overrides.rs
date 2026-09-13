//! User overrides: `overrides/<ICAO>/<layer>.geojson` files (WGS84) merged last.
//! A file with a top-level `"replace": true` replaces the generated layer entirely;
//! otherwise its features are appended. This is how layers with no free worldwide
//! source (hotspots, ATC blind spots, survey points) get filled per airport.

use crate::model::{AmdbFeature, Layer};
use crate::output::geojson::parse_feature_collection;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Default)]
pub struct Overrides {
    /// layer -> (replace?, features in WGS84)
    pub layers: HashMap<Layer, (bool, Vec<AmdbFeature>)>,
}

impl Overrides {
    pub fn load(root: &Path, icao: &str) -> Result<Overrides> {
        let mut out = Overrides::default();
        let dir = root.join(icao.to_uppercase());
        let Ok(rd) = std::fs::read_dir(&dir) else { return Ok(out) };
        for e in rd.flatten() {
            let p = e.path();
            let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else { continue };
            if p.extension().and_then(|s| s.to_str()).map_or(true, |x| !x.eq_ignore_ascii_case("geojson") && !x.eq_ignore_ascii_case("json")) {
                continue;
            }
            let Some(layer) = Layer::from_name(stem) else {
                log::warn!("override {}: unknown layer name", p.display());
                continue;
            };
            let text = std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
            let (replace, feats) = parse_feature_collection(&text, layer).with_context(|| format!("parse {}", p.display()))?;
            out.layers.insert(layer, (replace, feats));
        }
        Ok(out)
    }
}
