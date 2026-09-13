//! ParkingStandLocation and ParkingStandArea.

use super::Ctx;
use crate::geom::ops;
use crate::ir::{Stand, StandKind};
use crate::model::codes::{source, status};
use crate::model::feature::opt;
use crate::model::{AmdbFeature, Layer};
use geo_types::{Coord, Point, Polygon};

#[derive(Debug, Clone)]
pub struct StandLocal {
    pub name: String,
    pub pos: Coord<f64>,
    pub heading: Option<f64>,
    pub wingspan: f64,
    pub apron: Option<String>,
}

pub fn wingspan_for_code(c: char) -> f64 {
    match c {
        'A' => 15.0,
        'B' => 24.0,
        'C' => 36.0,
        'D' => 52.0,
        'E' => 65.0,
        'F' => 80.0,
        _ => 36.0,
    }
}

fn infer_code(s: &Stand) -> char {
    if let Some(c) = s.size_code {
        return c;
    }
    let types: Vec<&str> = s.aircraft_types.iter().map(String::as_str).collect();
    if types.contains(&"heavy") {
        'E'
    } else if types.contains(&"jets") {
        'C'
    } else if types.contains(&"turboprops") {
        'B'
    } else {
        match s.kind {
            StandKind::Gate => 'C',
            StandKind::Hangar | StandKind::Misc => 'B',
            StandKind::TieDown => 'A',
        }
    }
}

fn clearance_for_code(c: char) -> f64 {
    match c {
        'A' | 'B' => 3.0,
        'C' => 4.5,
        _ => 7.5,
    }
}

pub fn build(ctx: &mut Ctx) {
    let apron_polys: Vec<(String, Polygon<f64>)> = ctx
        .layer(Layer::ApronElement)
        .iter()
        .filter_map(|f| if let geo_types::Geometry::Polygon(p) = &f.geom { Some((f.get_str("idapron").unwrap_or("").to_string(), p.clone())) } else { None })
        .collect();
    let has_xp = ctx.src.stands.iter().any(|s| s.source == source::XPLANE);
    let mut chosen: Vec<(Stand, Coord<f64>)> = Vec::new();
    // X-Plane stands first; OSM stands only where no X-Plane stand is within 25 m.
    for s in ctx.src.stands.iter().filter(|s| s.source == source::XPLANE) {
        chosen.push((s.clone(), ctx.p(s.pos)));
    }
    for s in ctx.src.stands.iter().filter(|s| s.source != source::XPLANE) {
        let p = ctx.p(s.pos);
        if let Some(existing) = chosen.iter_mut().find(|(e, ep)| e.source == source::XPLANE && ops::dist(*ep, p) < 25.0) {
            // Enrich the X-Plane stand with OSM attributes it lacks.
            if existing.0.jetway.is_none() {
                existing.0.jetway = s.jetway;
            }
            if existing.0.area.is_none() {
                existing.0.area = s.area.clone();
            }
            continue;
        }
        if has_xp && s.name.is_empty() {
            continue;
        }
        chosen.push((s.clone(), p));
    }
    // Unique names.
    let mut seen = std::collections::HashMap::<String, usize>::new();
    let mut n_unnamed = 0;
    for (s, p) in chosen {
        let mut name = s.name.trim().to_string();
        if name.is_empty() {
            n_unnamed += 1;
            name = format!("STAND {n_unnamed}");
        }
        let c = seen.entry(name.clone()).or_insert(0);
        *c += 1;
        if *c > 1 {
            name = format!("{name} ({c})");
        }
        let code = infer_code(&s);
        let wingspan = wingspan_for_code(code);
        let apron = apron_polys.iter().find(|(_, poly)| ops::poly_contains_point(poly, p)).map(|(id, _)| id.clone());
        let acft = s.aircraft_types.join("|");
        ctx.push(
            AmdbFeature::new(Layer::ParkingStandLocation, Point(p))
                .with("idstd", name.clone())
                .with("idapron", opt(apron.clone()))
                .with("brngtrue", opt(s.heading_deg.map(super::runway::round1)))
                .with("wingspan", wingspan)
                .with("acft", if acft.is_empty() { serde_json::Value::Null } else { acft.into() })
                .with("jetway", opt(s.jetway))
                .with("gndpower", serde_json::Value::Null)
                .with("fuel", serde_json::Value::Null)
                .with("docking", serde_json::Value::Null)
                .with("towing", serde_json::Value::Null)
                .with("restacft", opt(s.operation.clone()))
                .with("airlines", if s.airlines.is_empty() { serde_json::Value::Null } else { s.airlines.join(" ").into() })
                .with("pmtyp", match s.kind {
                    StandKind::Gate => 1,
                    StandKind::Hangar => 2,
                    StandKind::Misc => 3,
                    StandKind::TieDown => 4,
                })
                .with("status", status::OPEN)
                .with("source", s.source),
        );
        // Stand area: OSM polygon or a wingspan-based box aligned with the heading.
        let area_poly = s
            .area
            .as_ref()
            .and_then(|ring| ops::tidy_polygon(&Polygon::new(ops::close_ring(ring.iter().map(|c| ctx.p(*c)).collect()), vec![])))
            .unwrap_or_else(|| {
                let heading = s.heading_deg.unwrap_or(0.0);
                let clearance = clearance_for_code(code);
                let w = wingspan + 2.0 * clearance;
                let l = (wingspan * 1.15).max(12.0);
                // The X-Plane start position is the aircraft reference point; bias the box
                // so roughly 40% lies ahead of the point.
                let u = ops::unit_from_heading(heading);
                let c = ops::add(p, ops::scale(u, -0.1 * l));
                ops::rect_centered(c, heading, l, w)
            });
        ctx.push(
            AmdbFeature::new(Layer::ParkingStandArea, area_poly)
                .with("idstd", name.clone())
                .with("idapron", opt(apron.clone()))
                .with("surftype", serde_json::Value::Null)
                .with("wingspan", wingspan)
                .with("status", status::OPEN)
                .with("source", if s.area.is_some() { s.source } else { source::DERIVED }),
        );
        ctx.stands_local.push(StandLocal { name, pos: p, heading: s.heading_deg, wingspan, apron });
    }
}
