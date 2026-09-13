//! Per-airport manifest and top-level index.

use crate::model::{Layer, ALL_LAYERS};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerInfo {
    pub count: usize,
    /// Sources that contributed features to this layer (e.g. ["xplane","derived"]).
    pub sources: Vec<String>,
    /// Why the layer is empty, when it is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub empty_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub icao: String,
    pub iata: Option<String>,
    pub name: Option<String>,
    pub country: Option<String>,
    pub arp: [f64; 2],
    pub elevation_ft: Option<f64>,
    pub projection: String,
    pub formats: Vec<String>,
    pub generated: String,
    pub generator: String,
    pub sources: Vec<String>,
    pub bbox: Option<[f64; 4]>,
    pub layers: BTreeMap<String, LayerInfo>,
    pub warnings: Vec<String>,
}

impl Manifest {
    pub fn write(&self, dir: &Path) -> anyhow::Result<()> {
        std::fs::write(dir.join("manifest.json"), serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn total_features(&self) -> usize {
        self.layers.values().map(|l| l.count).sum()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexAirport {
    pub icao: String,
    pub iata: Option<String>,
    pub name: Option<String>,
    pub country: Option<String>,
    pub arp: [f64; 2],
    pub bbox: Option<[f64; 4]>,
    pub features: usize,
    pub sources: Vec<String>,
    pub dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Index {
    pub generated: String,
    pub generator: String,
    pub projection: String,
    pub layers: Vec<String>,
    pub airports: Vec<IndexAirport>,
}

impl Index {
    pub fn load_or_new(root: &Path, projection: &str) -> Index {
        let p = root.join("index.json");
        let mut idx = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str::<Index>(&t).ok()).unwrap_or_default();
        idx.projection = projection.to_string();
        idx.layers = ALL_LAYERS.iter().map(|l: &Layer| l.name().to_string()).collect();
        idx.generator = format!("amdbgen {}", env!("CARGO_PKG_VERSION"));
        idx
    }

    pub fn upsert(&mut self, a: IndexAirport) {
        if let Some(e) = self.airports.iter_mut().find(|e| e.icao == a.icao) {
            *e = a;
        } else {
            self.airports.push(a);
        }
        self.airports.sort_by(|a, b| a.icao.cmp(&b.icao));
    }

    pub fn write(&mut self, root: &Path) -> anyhow::Result<()> {
        self.generated = chrono::Utc::now().to_rfc3339();
        std::fs::create_dir_all(root)?;
        std::fs::write(root.join("index.json"), serde_json::to_string_pretty(self)?)?;
        std::fs::write(root.join("codes.json"), serde_json::to_string_pretty(&crate::model::codes::legend())?)?;
        Ok(())
    }
}
