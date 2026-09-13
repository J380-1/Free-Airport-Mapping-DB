//! Conflation and derivation: turns a `SourceAirport` into AMDB layers.
//!
//! Order matters: runways first (everything else is clipped against them), then
//! pavements, painted lines, stands, the routing network, and finally structures and
//! point features. All geometry is in the local metre frame (`ctx.frame`).

pub mod asrn;
pub mod conv;
pub mod lines;
pub mod marking;
pub mod pavement;
pub mod runway;
pub mod signs;
pub mod stands;
pub mod structures;

use crate::geom::ops;
use crate::geom::LocalFrame;
use crate::ir::SourceAirport;
use crate::model::codes::source;
use crate::model::{AmdbFeature, Layer, ALL_LAYERS};
use geo::Centroid;
use geo_types::{Coord, Geometry, MultiPolygon, Polygon};
use std::collections::{BTreeMap, BTreeSet};

/// Tunables for the derivations.
#[derive(Debug, Clone)]
pub struct BuildOptions {
    /// Shoulder width used for derived runway/taxiway shoulders (metres, each side).
    pub runway_shoulder_m: f64,
    pub taxiway_shoulder_m: f64,
    /// Derive taxiway shoulders (synthetic) at all.
    pub derive_shoulders: bool,
    /// Generate Annex 14 runway markings.
    pub runway_markings: bool,
    /// Buffer for OSM taxiway centrelines without a width tag.
    pub default_taxiway_width_m: f64,
    /// Extent margin around pavement for keeping OSM structures (metres).
    pub extent_margin_m: f64,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            runway_shoulder_m: 7.5,
            taxiway_shoulder_m: 3.5,
            derive_shoulders: false,
            runway_markings: true,
            default_taxiway_width_m: 23.0,
            extent_margin_m: 200.0,
        }
    }
}

/// Runway geometry in the local frame, shared by later steps.
#[derive(Debug, Clone)]
pub struct RwyGeom {
    pub idrwy: String,
    pub idents: [String; 2],
    /// Physical ends.
    pub ends: [Coord<f64>; 2],
    /// Landing thresholds (displaced where applicable).
    pub thresholds: [Coord<f64>; 2],
    pub width: f64,
    pub length: f64,
    /// Heading from end 0 to end 1.
    pub heading: f64,
    pub poly: Polygon<f64>,
    pub surface: i64,
    pub src_index: usize,
}

pub struct Ctx<'a> {
    pub src: &'a SourceAirport,
    pub icao: String,
    pub frame: LocalFrame,
    pub opts: BuildOptions,
    pub out: BTreeMap<Layer, Vec<AmdbFeature>>,
    pub warnings: Vec<String>,
    pub layer_sources: BTreeMap<Layer, BTreeSet<String>>,
    pub runways: Vec<RwyGeom>,
    pub runway_mp: MultiPolygon<f64>,
    pub pavement_mp: MultiPolygon<f64>,
    pub taxiway_mp: MultiPolygon<f64>,
    pub apron_mp: MultiPolygon<f64>,
    /// Airport extent used to filter OSM features.
    pub extent: MultiPolygon<f64>,
    pub stands_local: Vec<stands::StandLocal>,
    pub asrn: asrn::Graph,
}

impl<'a> Ctx<'a> {
    pub fn p(&self, c: Coord<f64>) -> Coord<f64> {
        self.frame.forward(c.x, c.y)
    }

    pub fn push(&mut self, mut f: AmdbFeature) {
        f.props.entry("idarpt".to_string()).or_insert_with(|| self.icao.clone().into());
        let src = f.get_str("source").unwrap_or(source::DERIVED).to_string();
        self.layer_sources.entry(f.layer).or_default().insert(src);
        self.out.entry(f.layer).or_default().push(f);
    }

    pub fn warn(&mut self, msg: impl Into<String>) {
        let m = msg.into();
        log::debug!("{}: {m}", self.icao);
        self.warnings.push(m);
    }

    pub fn layer(&self, l: Layer) -> &[AmdbFeature] {
        self.out.get(&l).map(Vec::as_slice).unwrap_or(&[])
    }
}

pub struct BuildResult {
    pub frame: LocalFrame,
    pub features: BTreeMap<Layer, Vec<AmdbFeature>>,
    pub layer_sources: BTreeMap<Layer, Vec<String>>,
    pub warnings: Vec<String>,
}

/// Determine the ARP: explicit datum, else midpoint of the longest runway, else the
/// centroid of everything, else the OSM aerodrome polygon centroid.
pub fn choose_arp(src: &SourceAirport) -> Option<(f64, f64)> {
    if let Some(c) = src.header.arp {
        return Some((c.y, c.x));
    }
    if let Some(r) = src.runways.iter().max_by(|a, b| {
        let la = (a.ends[0].pos.x - a.ends[1].pos.x).hypot(a.ends[0].pos.y - a.ends[1].pos.y);
        let lb = (b.ends[0].pos.x - b.ends[1].pos.x).hypot(b.ends[0].pos.y - b.ends[1].pos.y);
        la.partial_cmp(&lb).unwrap()
    }) {
        return Some(((r.ends[0].pos.y + r.ends[1].pos.y) / 2.0, (r.ends[0].pos.x + r.ends[1].pos.x) / 2.0));
    }
    src.bbox().map(|(a, b, c, d)| ((b + d) / 2.0, (a + c) / 2.0))
}

pub fn build(src: &SourceAirport, opts: BuildOptions) -> anyhow::Result<BuildResult> {
    let (lat0, lon0) = choose_arp(src).ok_or_else(|| anyhow::anyhow!("no position known for {}", src.header.icao))?;
    let frame = LocalFrame::new(lat0, lon0);
    let mut ctx = Ctx {
        src,
        icao: src.header.icao.clone(),
        frame,
        opts,
        out: BTreeMap::new(),
        warnings: Vec::new(),
        layer_sources: BTreeMap::new(),
        runways: Vec::new(),
        runway_mp: MultiPolygon(vec![]),
        pavement_mp: MultiPolygon(vec![]),
        taxiway_mp: MultiPolygon(vec![]),
        apron_mp: MultiPolygon(vec![]),
        extent: MultiPolygon(vec![]),
        stands_local: Vec::new(),
        asrn: asrn::Graph::default(),
    };
    for l in ALL_LAYERS {
        ctx.out.entry(*l).or_default();
    }
    let icao = ctx.icao.clone();
    let phase = |ctx: &mut Ctx, name: &str, f: fn(&mut Ctx)| {
        let t = std::time::Instant::now();
        f(ctx);
        crate::term::step(Some(&icao), &format!("{name} in {}", crate::term::human_secs(t.elapsed().as_secs_f64())));
    };
    phase(&mut ctx, "Aerodrome reference point", structures::aerodrome_reference_point);
    phase(&mut ctx, "Runways, thresholds, intersections, shoulders", runway::build);
    if ctx.opts.runway_markings {
        phase(&mut ctx, "Runway markings (Annex 14)", marking::build);
    }
    phase(&mut ctx, "Taxiway and apron pavement", pavement::build);
    phase(&mut ctx, "Airport extent", structures::compute_extent);
    phase(&mut ctx, "Parking stands", stands::build);
    phase(&mut ctx, "Guidance lines, holds, exits", lines::build);
    phase(&mut ctx, "Routing network (ASRN)", asrn::build);
    phase(&mut ctx, "Buildings, roads, water, frequencies, helipads", structures::build);
    phase(&mut ctx, "Signs", signs::build);
    assign_ids(&mut ctx);
    let n = ALL_LAYERS.len();
    for (i, l) in ALL_LAYERS.iter().enumerate() {
        let count = ctx.out.get(l).map(Vec::len).unwrap_or(0);
        crate::term::layer(Some(&icao), i + 1, n, l.name(), count);
    }
    for w in &src.warnings {
        ctx.warnings.push(w.clone());
    }
    Ok(BuildResult {
        frame,
        features: ctx.out,
        layer_sources: ctx.layer_sources.into_iter().map(|(k, v)| (k, v.into_iter().collect())).collect(),
        warnings: ctx.warnings,
    })
}

fn sort_key(g: &Geometry<f64>) -> (i64, i64) {
    let c = match g {
        Geometry::Point(p) => Some(p.0),
        Geometry::LineString(l) => l.centroid().map(|p| p.0),
        Geometry::Polygon(p) => p.centroid().map(|p| p.0),
        Geometry::MultiPolygon(m) => m.centroid().map(|p| p.0),
        Geometry::MultiLineString(m) => m.centroid().map(|p| p.0),
        _ => None,
    }
    .unwrap_or(Coord { x: 0.0, y: 0.0 });
    ((c.x * 10.0).round() as i64, (c.y * 10.0).round() as i64)
}

/// Deterministic ids: `<ICAO>:<layer>:<n>` in canonical (spatially sorted) order.
/// Features that already carry an id (overrides) keep it.
pub fn assign_ids(ctx: &mut Ctx) {
    let icao = ctx.icao.clone();
    for (layer, feats) in ctx.out.iter_mut() {
        feats.sort_by_cached_key(|f| (sort_key(&f.geom), f.get_str("idlin").unwrap_or("").to_string()));
        for (i, f) in feats.iter_mut().enumerate() {
            if !f.props.contains_key("id") {
                f.set("id", format!("{icao}:{}:{}", layer.name(), i + 1));
            }
            f.props.entry("source".to_string()).or_insert_with(|| source::DERIVED.into());
        }
    }
}

/// Convenience: polygon list -> MultiPolygon union.
pub fn union(polys: &[Polygon<f64>]) -> MultiPolygon<f64> {
    ops::union_all(polys)
}
