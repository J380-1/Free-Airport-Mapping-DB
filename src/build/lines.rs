//! Painted lines: TaxiwayGuidanceLine, TaxiwayHoldingPosition,
//! TaxiwayIntersectionMarking, StandGuidanceLine, RunwayExitLine, and roadway lines
//! turned into ServiceRoad surfaces when OSM has no roads.

use super::{conv, Ctx};
use crate::geom::ops::{self, add, scale};
use crate::ir::LineKind;
use crate::model::codes::{catstop, source};
use crate::model::feature::opt;
use crate::model::{AmdbFeature, Layer};
use geo_types::{Coord, LineString};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sem {
    Center,
    CenterIls,
    Lane,
    RunwayHold,
    IlsHold,
    IntersectionHold,
    RoadCenter,
    Ignore,
}

/// Known apt.dat line codes. X-Plane 12 added undocumented codes; 60/62/64 were
/// verified against real scenery geometry (centreline / runway hold / ILS hold).
/// Codes that return `None` are classified geometrically by `classify_geom`.
fn semantic(code: u16) -> Option<Sem> {
    Some(match code {
        0 => Sem::Ignore,
        1 | 51 | 60 => Sem::Center,
        7 | 57 => Sem::CenterIls,
        8 | 9 | 58 | 59 => Sem::Lane,
        4 | 54 | 62 => Sem::RunwayHold,
        6 | 56 | 64 => Sem::IlsHold,
        5 | 55 | 63 => Sem::IntersectionHold,
        3 | 53 | 30 | 31 | 32 => Sem::Ignore, // edge lines
        20 | 21 | 23 | 24 | 25 => Sem::Ignore, // roadway edge / chequer
        22 => Sem::RoadCenter,
        2 | 52 => return None, // broken yellow: ICAO intermediate hold when it crosses a taxi route
        _ => return None,
    })
}

/// Geometric classification for codes without a known meaning.
fn classify_geom(pts: &LineString<f64>, edges: &[EdgeRef], runways: &[super::RwyGeom]) -> Sem {
    let len = ops::length(pts);
    if pts.0.len() < 2 || len < 1.0 {
        return Sem::Ignore;
    }
    let mid = ops::point_at(pts, len / 2.0);
    let a = pts.0[0];
    let b = *pts.0.last().unwrap();
    // Nearest routing edge to the midpoint.
    let mut best: Option<(f64, &EdgeRef)> = None;
    for e in edges {
        let d = ops::point_seg_dist(mid, e.a, e.b);
        if best.map_or(true, |(bd, _)| d < bd) {
            best = Some((d, e));
        }
    }
    let Some((d, e)) = best else { return Sem::Ignore };
    if d > 4.0 {
        return Sem::Ignore;
    }
    let ang = |p: Coord<f64>, q: Coord<f64>| (q.y - p.y).atan2(q.x - p.x).to_degrees();
    let mut diff = (ang(a, b) - ang(e.a, e.b)).abs() % 180.0;
    if diff > 90.0 {
        diff = 180.0 - diff;
    }
    if diff > 60.0 && len < 90.0 {
        let near_rwy = runways.iter().any(|r| ops::point_seg_dist(mid, r.ends[0], r.ends[1]) < 350.0);
        if near_rwy { Sem::RunwayHold } else { Sem::IntersectionHold }
    } else if diff < 30.0 && len >= 5.0 {
        Sem::Center
    } else {
        Sem::Ignore
    }
}

fn lit(light: u16) -> bool {
    matches!(light, 101 | 105 | 107 | 108)
}

struct EdgeRef {
    a: Coord<f64>,
    b: Coord<f64>,
    name: String,
    wingspan: Option<f64>,
}

fn nearest_edge_name(line: &LineString<f64>, edges: &[EdgeRef], max_d: f64) -> (Option<String>, Option<f64>) {
    // Sample a few points along the line and vote.
    let n = line.0.len();
    let samples: Vec<Coord<f64>> = if n <= 3 { line.0.clone() } else { vec![line.0[0], line.0[n / 2], line.0[n - 1]] };
    let mut votes: HashMap<&str, (usize, Option<f64>)> = HashMap::new();
    for s in samples {
        let mut best: Option<(f64, &EdgeRef)> = None;
        for e in edges {
            let d = ops::point_seg_dist(s, e.a, e.b);
            if d <= max_d && best.map_or(true, |(bd, _)| d < bd) {
                best = Some((d, e));
            }
        }
        if let Some((_, e)) = best {
            let v = votes.entry(e.name.as_str()).or_insert((0, e.wingspan));
            v.0 += 1;
        }
    }
    votes.into_iter().max_by_key(|(n, (c, _))| (*c, std::cmp::Reverse(n.to_string()))).map(|(n, (_, w))| (Some(n.to_string()), w)).unwrap_or((None, None))
}

pub fn wingspan_for_restriction(r: &str) -> Option<f64> {
    match r.to_ascii_uppercase().as_str() {
        "TAXIWAY_A" => Some(15.0),
        "TAXIWAY_B" => Some(24.0),
        "TAXIWAY_C" => Some(36.0),
        "TAXIWAY_D" => Some(52.0),
        "TAXIWAY_E" => Some(65.0),
        "TAXIWAY_F" => Some(80.0),
        _ => None,
    }
}

pub fn build(ctx: &mut Ctx) {
    let node_pos: HashMap<i64, Coord<f64>> = ctx.src.route_nodes.iter().map(|n| (n.id, ctx.p(n.pos))).collect();
    let edges: Vec<EdgeRef> = ctx
        .src
        .route_edges
        .iter()
        .filter(|e| e.restriction != "runway")
        .filter_map(|e| Some(EdgeRef { a: *node_pos.get(&e.from)?, b: *node_pos.get(&e.to)?, name: e.name.clone()?, wingspan: wingspan_for_restriction(&e.restriction) }))
        .collect();
    let all_edges: Vec<EdgeRef> = ctx
        .src
        .route_edges
        .iter()
        .filter(|e| e.restriction != "runway")
        .filter_map(|e| Some(EdgeRef { a: *node_pos.get(&e.from)?, b: *node_pos.get(&e.to)?, name: e.name.clone().unwrap_or_default(), wingspan: None }))
        .collect();
    let stands: Vec<(String, Coord<f64>)> = ctx.stands_local.iter().map(|s| (s.name.clone(), s.pos)).collect();
    let runway_mp = ctx.runway_mp.clone();
    let runways = ctx.runways.clone();

    // Collect coded runs from painted lines and pavement edges.
    let mut runs: Vec<(conv::CodedRun, Option<String>, &'static str)> = Vec::new();
    for l in &ctx.src.painted_lines {
        for r in conv::coded_runs(&ctx.frame, &l.ring) {
            runs.push((r, l.name.clone(), l.source));
        }
    }
    for pv in &ctx.src.pavements {
        for r in conv::coded_runs(&ctx.frame, &pv.outer) {
            runs.push((r, None, pv.source));
        }
        for h in &pv.holes {
            for r in conv::coded_runs(&ctx.frame, h) {
                runs.push((r, None, pv.source));
            }
        }
    }
    let has_xp_lines = runs.iter().any(|(r, _, _)| matches!(semantic(r.line), Some(Sem::Center) | Some(Sem::CenterIls)));

    let mut road_lines: Vec<LineString<f64>> = Vec::new();
    for (run, desc, src) in runs {
        let sem = semantic(run.line).unwrap_or_else(|| classify_geom(&run.pts, &all_edges, &runways));
        let lighting = lit(run.light);
        match sem {
            Sem::Ignore => {}
            Sem::RoadCenter => road_lines.push(run.pts),
            Sem::RunwayHold | Sem::IlsHold => {
                let cs = if sem == Sem::RunwayHold { catstop::CAT_I } else { catstop::CAT_II_III };
                let idrwy = nearest_runway(&run.pts, &runways, 500.0);
                let (idlin, _) = nearest_edge_name(&run.pts, &edges, 40.0);
                ctx.push(AmdbFeature::new(Layer::TaxiwayHoldingPosition, run.pts).with("idlin", opt(idlin)).with("idrwy", opt(idrwy)).with("catstop", cs).with("lighting", matches!(run.light, 103 | 104)).with("status", 1).with("source", src));
            }
            Sem::IntersectionHold => {
                let (idlin, _) = nearest_edge_name(&run.pts, &edges, 40.0);
                ctx.push(AmdbFeature::new(Layer::TaxiwayIntersectionMarking, run.pts).with("idlin", opt(idlin)).with("source", src));
            }
            Sem::Center | Sem::CenterIls | Sem::Lane => {
                emit_centerline(ctx, run.pts, sem == Sem::Lane, sem == Sem::CenterIls, lighting, desc, src, &edges, &stands, &runway_mp, &runways);
            }
        }
    }

    // OSM fallback centrelines when X-Plane painted none.
    if !has_xp_lines {
        for l in ctx.src.semantic_lines.iter().filter(|l| l.kind == LineKind::TaxiCenterline) {
            let ls = ctx.frame.fwd_line(&l.pts);
            if ls.0.len() < 2 {
                continue;
            }
            let name = l.name.clone();
            emit_centerline(ctx, ls, false, false, false, name, source::OSM, &edges, &stands, &runway_mp, &runways);
        }
    }
    // OSM holding-position nodes -> short lines perpendicular to the nearest centreline.
    let centerlines: Vec<LineString<f64>> = ctx.layer(Layer::TaxiwayGuidanceLine).iter().filter_map(|f| if let geo_types::Geometry::LineString(l) = &f.geom { Some(l.clone()) } else { None }).collect();
    let holds: Vec<(Coord<f64>, LineKind, Option<String>)> = ctx.src.semantic_lines.iter().filter(|l| matches!(l.kind, LineKind::RunwayHold | LineKind::IlsHold | LineKind::IntersectionHold) && l.pts.len() == 1).map(|l| (ctx.p(l.pts[0]), l.kind, l.name.clone())).collect();
    let existing_holds: Vec<LineString<f64>> = ctx.layer(Layer::TaxiwayHoldingPosition).iter().filter_map(|f| if let geo_types::Geometry::LineString(l) = &f.geom { Some(l.clone()) } else { None }).collect();
    for (p, kind, name) in holds {
        if existing_holds.iter().any(|h| ops::point_line_dist(p, h) < 15.0) {
            continue; // X-Plane already painted this one
        }
        let dir = nearest_direction(p, &centerlines).unwrap_or(Coord { x: 1.0, y: 0.0 });
        let r = ops::right_of(dir);
        let half = 12.0;
        let ls = LineString(vec![add(p, scale(r, -half)), add(p, scale(r, half))]);
        match kind {
            LineKind::IntersectionHold => ctx.push(AmdbFeature::new(Layer::TaxiwayIntersectionMarking, ls).with("idlin", opt(name)).with("source", source::OSM)),
            _ => {
                let cs = if kind == LineKind::IlsHold { catstop::CAT_II_III } else { catstop::CAT_I };
                let idrwy = nearest_runway(&ls, &runways, 500.0);
                ctx.push(AmdbFeature::new(Layer::TaxiwayHoldingPosition, ls).with("idlin", opt(name)).with("idrwy", opt(idrwy)).with("catstop", cs).with("lighting", false).with("status", 1).with("source", source::OSM));
            }
        }
    }
    // Roadway centrelines (X-Plane) -> service roads if OSM has none.
    let has_osm_roads = ctx.src.semantic_lines.iter().any(|l| l.kind == LineKind::RoadCenter);
    if !has_osm_roads {
        for ls in road_lines {
            for p in ops::tidy_multi(&ops::buffer_line(&ls, 3.0)) {
                ctx.push(AmdbFeature::new(Layer::ServiceRoad, p).with("name", serde_json::Value::Null).with("surftype", 4).with("width", 6.0).with("bridge", false).with("source", source::XPLANE));
            }
        }
    }
}

fn nearest_direction(p: Coord<f64>, lines: &[LineString<f64>]) -> Option<Coord<f64>> {
    let mut best: Option<(f64, Coord<f64>)> = None;
    for l in lines {
        for w in l.0.windows(2) {
            let d = ops::point_seg_dist(p, w[0], w[1]);
            if best.map_or(true, |(bd, _)| d < bd) {
                let len = ops::dist(w[0], w[1]);
                if len > 0.01 {
                    best = Some((d, Coord { x: (w[1].x - w[0].x) / len, y: (w[1].y - w[0].y) / len }));
                }
            }
        }
    }
    best.filter(|(d, _)| *d < 60.0).map(|(_, u)| u)
}

fn nearest_runway(line: &LineString<f64>, runways: &[super::RwyGeom], max_d: f64) -> Option<String> {
    let mid = ops::point_at(line, ops::length(line) / 2.0);
    runways
        .iter()
        .map(|r| (ops::point_seg_dist(mid, r.ends[0], r.ends[1]), &r.idrwy))
        .filter(|(d, _)| *d <= max_d)
        .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap())
        .map(|(_, id)| id.clone())
}

#[allow(clippy::too_many_arguments)]
fn emit_centerline(
    ctx: &mut Ctx,
    pts: LineString<f64>,
    lane: bool,
    ils_critical: bool,
    lighting: bool,
    desc: Option<String>,
    src: &'static str,
    edges: &[EdgeRef],
    stands: &[(String, Coord<f64>)],
    runway_mp: &geo_types::MultiPolygon<f64>,
    runways: &[super::RwyGeom],
) {
    // Portions inside a runway are exit lines.
    let (inside, outside) = if runway_mp.0.is_empty() { (vec![], vec![pts]) } else { ops::split_by(runway_mp, &pts) };
    for seg in inside {
        let idrwy = nearest_runway(&seg, runways, 200.0);
        let (idlin, _) = nearest_edge_name(&seg, edges, 60.0);
        let exittype = runways
            .iter()
            .find(|r| Some(&r.idrwy) == idrwy.as_ref())
            .map(|r| {
                let a = ops::heading_deg(seg.0[0], *seg.0.last().unwrap());
                let mut diff = (a - r.heading).abs() % 180.0;
                if diff > 90.0 {
                    diff = 180.0 - diff;
                }
                if (20.0..=50.0).contains(&diff) { 2 } else { 1 }
            })
            .unwrap_or(1);
        ctx.push(AmdbFeature::new(Layer::RunwayExitLine, seg).with("idlin", opt(idlin)).with("idrwy", opt(idrwy)).with("exittype", exittype).with("lighting", lighting).with("source", src));
    }
    for seg in outside {
        let len = ops::length(&seg);
        let first = seg.0[0];
        let last = *seg.0.last().unwrap();
        // Stand lead-in: short line ending at a stand.
        let stand = stands.iter().filter(|(_, p)| ops::dist(*p, first).min(ops::dist(*p, last)) < 20.0).min_by(|a, b| ops::dist(a.1, last).min(ops::dist(a.1, first)).partial_cmp(&ops::dist(b.1, last).min(ops::dist(b.1, first))).unwrap());
        if let (Some((idstd, _)), true) = (stand, len < 250.0) {
            ctx.push(AmdbFeature::new(Layer::StandGuidanceLine, seg).with("idstd", idstd.clone()).with("idlin", serde_json::Value::Null).with("color", 1).with("style", 1).with("source", src));
            continue;
        }
        let (idlin, wingspan) = nearest_edge_name(&seg, edges, 40.0);
        let idlin = idlin.or_else(|| desc.clone().filter(|d| d.len() <= 5 && !d.to_ascii_lowercase().contains("line")));
        ctx.push(
            AmdbFeature::new(Layer::TaxiwayGuidanceLine, seg)
                .with("idlin", opt(idlin))
                .with("color", 1)
                .with("style", if lane { 2 } else { 1 })
                .with("direc", 1)
                .with("lighting", lighting)
                .with("ilscrit", ils_critical)
                .with("wingspan", opt(wingspan))
                .with("length", super::runway::round1(len))
                .with("source", src),
        );
    }
}
