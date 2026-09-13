//! TaxiwayElement / ApronElement / TaxiwayShoulder (+ ServiceRoad and FATO from
//! X-Plane pavements whose description says so).

use super::{conv, Ctx};
use crate::geom::ops;
use crate::ir::{AreaKind, LineKind, PavementHint};
use crate::model::codes::{apron_feattype, source, status, surftype, twy_feattype};
use crate::model::{AmdbFeature, Layer};
use geo::{Area, BooleanOps, Intersects};
use geo_types::{Coord, LineString, MultiPolygon, Polygon};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Taxiway,
    Apron,
    Road,
    Helipad,
}

struct Piece {
    poly: Polygon<f64>,
    class: Class,
    surface: i64,
    name: Option<String>,
    source: &'static str,
    bridge: bool,
}

fn is_paved(s: i64) -> bool {
    matches!(s, surftype::ASPHALT | surftype::ASPHALT_GROOVED | surftype::CONCRETE | surftype::CONCRETE_GROOVED | surftype::BRICK_PAVERS | surftype::MACADAM | surftype::AGGREGATE_SEAL)
}

fn ring_from(ctx: &Ctx, outer: &[Coord<f64>], holes: &[Vec<Coord<f64>>]) -> Option<Polygon<f64>> {
    let ext = ops::close_ring(outer.iter().map(|c| ctx.p(*c)).collect());
    if ext.0.len() < 4 {
        return None;
    }
    let ints: Vec<LineString<f64>> = holes.iter().map(|h| ops::close_ring(h.iter().map(|c| ctx.p(*c)).collect())).filter(|l| l.0.len() >= 4).collect();
    ops::tidy_polygon(&Polygon::new(ext, ints))
}

pub fn build(ctx: &mut Ctx) {
    let stands: Vec<Coord<f64>> = ctx.src.stands.iter().map(|s| ctx.p(s.pos)).collect();
    // Route edge midpoints with names for taxiway naming and classification.
    let node_pos: HashMap<i64, Coord<f64>> = ctx.src.route_nodes.iter().map(|n| (n.id, ctx.p(n.pos))).collect();
    let edge_mids: Vec<(Coord<f64>, Option<String>, bool)> = ctx
        .src
        .route_edges
        .iter()
        .filter_map(|e| {
            let a = node_pos.get(&e.from)?;
            let b = node_pos.get(&e.to)?;
            Some((Coord { x: (a.x + b.x) / 2.0, y: (a.y + b.y) / 2.0 }, e.name.clone(), e.restriction == "runway"))
        })
        .collect();
    let osm_aprons: Vec<Polygon<f64>> = ctx.src.areas.iter().filter(|a| a.kind == AreaKind::Apron || a.kind == AreaKind::Deicing).filter_map(|a| ring_from(ctx, &a.outer, &a.holes)).collect();
    let osm_apron_mp = ops::union_all(&osm_aprons);
    let bridge_lines: Vec<LineString<f64>> = ctx.src.semantic_lines.iter().filter(|l| l.bridge && l.kind == LineKind::TaxiCenterline).map(|l| ctx.frame.fwd_line(&l.pts)).collect();

    let mut pieces: Vec<Piece> = Vec::new();
    let has_xp_pavement = ctx.src.pavements.iter().any(|p| p.source == source::XPLANE);

    for pv in &ctx.src.pavements {
        let Some(poly) = conv::ring_to_polygon(&ctx.frame, &pv.outer, &pv.holes) else { continue };
        let class = match pv.hint {
            PavementHint::Apron => Class::Apron,
            PavementHint::Road => Class::Road,
            PavementHint::Helipad => Class::Helipad,
            PavementHint::Taxiway => Class::Taxiway,
            PavementHint::Runway | PavementHint::Unknown => {
                let has_stand = stands.iter().any(|s| ops::poly_contains_point(&poly, *s));
                let twy_edges = edge_mids.iter().filter(|(m, _, rw)| !rw && ops::poly_contains_point(&poly, *m)).count();
                if has_stand && twy_edges == 0 {
                    Class::Apron
                } else if twy_edges > 0 && !has_stand {
                    Class::Taxiway
                } else if !osm_apron_mp.0.is_empty() {
                    let inter = poly.intersection(&osm_apron_mp).unsigned_area();
                    if inter > 0.5 * poly.unsigned_area() { Class::Apron } else { Class::Taxiway }
                } else if has_stand {
                    Class::Apron
                } else if poly.unsigned_area() > 40_000.0 && twy_edges == 0 {
                    Class::Apron
                } else {
                    Class::Taxiway
                }
            }
        };
        let bridge = bridge_lines.iter().any(|l| poly.intersects(l));
        pieces.push(Piece { poly, class, surface: pv.surface, name: pv.name.clone(), source: pv.source, bridge });
    }

    // OSM fallback / supplement.
    if !has_xp_pavement {
        for a in &ctx.src.areas {
            let class = match a.kind {
                AreaKind::Apron | AreaKind::Deicing => Class::Apron,
                AreaKind::Taxiway => Class::Taxiway,
                AreaKind::Helipad => Class::Helipad,
                _ => continue,
            };
            if let Some(poly) = ring_from(ctx, &a.outer, &a.holes) {
                pieces.push(Piece { poly, class, surface: a.surface.unwrap_or(surftype::UNKNOWN), name: a.name.clone(), source: source::OSM, bridge: false });
            }
        }
        // Buffer OSM taxiway centrelines into pavement.
        let mut twy_polys: Vec<(Polygon<f64>, Option<String>, bool)> = Vec::new();
        for l in ctx.src.semantic_lines.iter().filter(|l| l.kind == LineKind::TaxiCenterline) {
            let ls = ctx.frame.fwd_line(&l.pts);
            let w = l.width_m.unwrap_or(ctx.opts.default_taxiway_width_m);
            for p in ops::buffer_line(&ls, w / 2.0).0 {
                twy_polys.push((p, l.name.clone(), l.bridge));
            }
        }
        for (p, name, bridge) in twy_polys {
            pieces.push(Piece { poly: p, class: Class::Taxiway, surface: surftype::ASPHALT, name, source: source::OSM, bridge });
        }
        if pieces.is_empty() {
            ctx.warn("no taxiway/apron pavement in any source");
        }
    } else {
        // Add OSM aprons not covered by X-Plane pavement (X-Plane wins where both exist).
        let xp_mp = ops::union_all(&pieces.iter().map(|p| p.poly.clone()).collect::<Vec<_>>());
        for a in ctx.src.areas.iter().filter(|a| a.kind == AreaKind::Apron) {
            if let Some(poly) = ring_from(ctx, &a.outer, &a.holes) {
                let rest = if xp_mp.0.is_empty() { MultiPolygon(vec![poly]) } else { poly.difference(&xp_mp) };
                for p in ops::tidy_multi(&rest) {
                    if p.unsigned_area() > 500.0 {
                        pieces.push(Piece { poly: p, class: Class::Apron, surface: a.surface.unwrap_or(surftype::UNKNOWN), name: a.name.clone(), source: source::OSM, bridge: false });
                    }
                }
            }
        }
    }

    // Subtract runways from taxiway/apron pieces and emit.
    let runway_mp = ctx.runway_mp.clone();
    let mut taxi_polys = Vec::new();
    let mut apron_polys = Vec::new();
    let mut apron_n = 0;
    for piece in pieces {
        let parts: Vec<Polygon<f64>> = if runway_mp.0.is_empty() || !piece.poly.intersects(&runway_mp) {
            vec![piece.poly.clone()]
        } else {
            ops::tidy_multi(&piece.poly.difference(&runway_mp))
        };
        for p in parts {
            if p.unsigned_area() < 2.0 {
                continue;
            }
            match piece.class {
                Class::Taxiway => {
                    let idlin = piece.name.clone().filter(|n| n.len() <= 6).or_else(|| dominant_name(&p, &edge_mids));
                    let touches_runway = !runway_mp.0.is_empty() && ops::buffer_polygon(&p, 1.0).intersects(&runway_mp);
                    let feattype = if piece.bridge { twy_feattype::BRIDGE } else if touches_runway { twy_feattype::EXIT } else { twy_feattype::UNKNOWN };
                    taxi_polys.push(p.clone());
                    ctx.push(AmdbFeature::new(Layer::TaxiwayElement, p).with("idlin", crate::model::feature::opt(idlin)).with("surftype", piece.surface).with("feattype", feattype).with("bridge", piece.bridge).with("status", status::OPEN).with("pcn", serde_json::Value::Null).with("source", piece.source));
                }
                Class::Apron => {
                    apron_n += 1;
                    let idapron = piece.name.clone().unwrap_or_else(|| format!("APRON {apron_n}"));
                    let ft = apron_kind(&idapron);
                    apron_polys.push(p.clone());
                    ctx.push(AmdbFeature::new(Layer::ApronElement, p).with("idapron", idapron).with("surftype", piece.surface).with("feattype", ft).with("status", status::OPEN).with("pcn", serde_json::Value::Null).with("source", piece.source));
                }
                Class::Road => {
                    ctx.push(AmdbFeature::new(Layer::ServiceRoad, p).with("name", crate::model::feature::opt(piece.name.clone())).with("surftype", piece.surface).with("width", serde_json::Value::Null).with("bridge", false).with("source", piece.source));
                }
                Class::Helipad => {
                    ctx.push(AmdbFeature::new(Layer::FinalApproachAndTakeOffArea, p).with("ident", crate::model::feature::opt(piece.name.clone())).with("surftype", piece.surface).with("length", serde_json::Value::Null).with("width", serde_json::Value::Null).with("brngtrue", serde_json::Value::Null).with("source", piece.source));
                }
            }
        }
    }
    ctx.taxiway_mp = ops::union_all(&taxi_polys);
    ctx.apron_mp = ops::union_all(&apron_polys);
    ctx.pavement_mp = ops::mp_union(&ctx.taxiway_mp, &ctx.apron_mp);

    // Derived taxiway shoulders: a band around paved taxiways/aprons not covered by
    // any other pavement.
    if ctx.opts.derive_shoulders && !ctx.pavement_mp.0.is_empty() {
        let w = ctx.opts.taxiway_shoulder_m;
        let band = ops::mp_difference(&ops::buffer_multi(&ctx.pavement_mp, w), &ctx.pavement_mp);
        let rw_sh: Vec<Polygon<f64>> = ctx.layer(Layer::RunwayShoulder).iter().filter_map(|f| if let geo_types::Geometry::Polygon(p) = &f.geom { Some(p.clone()) } else { None }).collect();
        let mut minus = ops::mp_union(&runway_mp, &ops::union_all(&rw_sh));
        let blast: Vec<Polygon<f64>> = ctx.layer(Layer::Blastpad).iter().chain(ctx.layer(Layer::Stopway).iter()).chain(ctx.layer(Layer::RunwayDisplacedArea).iter()).filter_map(|f| if let geo_types::Geometry::Polygon(p) = &f.geom { Some(p.clone()) } else { None }).collect();
        minus = ops::mp_union(&minus, &ops::union_all(&blast));
        let band = ops::mp_difference(&band, &minus);
        for p in ops::tidy_multi(&band) {
            if p.unsigned_area() < 5.0 {
                continue;
            }
            let idlin = dominant_name(&p, &edge_mids);
            ctx.push(AmdbFeature::new(Layer::TaxiwayShoulder, p).with("idlin", crate::model::feature::opt(idlin)).with("surftype", surftype::UNKNOWN).with("width", w).with("source", source::DERIVED));
        }
    }
    let _ = is_paved;
}

fn dominant_name(p: &Polygon<f64>, mids: &[(Coord<f64>, Option<String>, bool)]) -> Option<String> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for (m, name, rw) in mids {
        if *rw {
            continue;
        }
        if let Some(n) = name {
            if ops::poly_contains_point(p, *m) {
                *counts.entry(n.as_str()).or_default() += 1;
            }
        }
    }
    counts.into_iter().max_by_key(|(n, c)| (*c, std::cmp::Reverse(n.to_string()))).map(|(n, _)| n.to_string())
}

fn apron_kind(name: &str) -> i64 {
    let n = name.to_ascii_lowercase();
    if n.contains("cargo") || n.contains("freight") {
        apron_feattype::CARGO
    } else if n.contains("ga ") || n.starts_with("ga") || n.contains("general") || n.contains("tie") {
        apron_feattype::GA
    } else if n.contains("maint") || n.contains("mro") || n.contains("hangar") {
        apron_feattype::MAINTENANCE
    } else if n.contains("mil") {
        apron_feattype::MILITARY
    } else if n.contains("deic") || n.contains("de-ic") {
        apron_feattype::DEICING
    } else if n.contains("fuel") {
        apron_feattype::FUEL
    } else if n.contains("heli") {
        apron_feattype::HELICOPTER
    } else if n.contains("gate") || n.contains("terminal") || n.contains("apron") || n.contains("ramp") || n.contains("stand") {
        apron_feattype::PARKING
    } else {
        apron_feattype::UNKNOWN
    }
}
