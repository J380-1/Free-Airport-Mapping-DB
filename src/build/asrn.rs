//! Aerodrome Surface Routing Network (AsrnNode / AsrnEdge).
//!
//! Primary source is the X-Plane taxi routing network (rows 1201/1202/1204). Without
//! it, a graph is built from OSM taxiway centrelines and the runway centrelines.
//! Stands are attached by projecting each stand onto the nearest taxiway edge.

use super::lines::wingspan_for_restriction;
use super::Ctx;
use crate::geom::ops::{self, dist};
use crate::ir::LineKind;
use crate::model::codes::{direc, edgetype, nodetype, source};
use crate::model::feature::opt;
use crate::model::{AmdbFeature, Layer};
use geo_types::{Coord, Line, LineString, Point};
use std::collections::{BTreeSet, HashMap};

#[derive(Debug, Clone)]
pub struct GNode {
    pub pos: Coord<f64>,
    pub nodetype: i64,
    pub idrwy: Option<String>,
    pub idstd: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GEdge {
    pub a: usize,
    pub b: usize,
    pub edgetype: i64,
    pub direc: i64,
    pub name: Option<String>,
    pub wingspan: Option<f64>,
    pub idrwy: Option<String>,
    pub active: Vec<String>,
    pub pts: LineString<f64>,
    pub source: &'static str,
}

#[derive(Debug, Clone, Default)]
pub struct Graph {
    pub nodes: Vec<GNode>,
    pub edges: Vec<GEdge>,
}

impl Graph {
    fn add_node(&mut self, pos: Coord<f64>) -> usize {
        self.nodes.push(GNode { pos, nodetype: nodetype::TAXIWAY, idrwy: None, idstd: None });
        self.nodes.len() - 1
    }

    fn find_node_near(&self, p: Coord<f64>, tol: f64) -> Option<usize> {
        self.nodes.iter().enumerate().filter(|(_, n)| dist(n.pos, p) <= tol).min_by(|a, b| dist(a.1.pos, p).partial_cmp(&dist(b.1.pos, p)).unwrap()).map(|(i, _)| i)
    }

    /// Split edge `ei` at the point nearest `p`; returns the node at the split.
    fn split_edge_at(&mut self, ei: usize, p: Coord<f64>) -> usize {
        let e = self.edges[ei].clone();
        // Locate nearest segment and projection.
        let mut best = (f64::INFINITY, 0usize, p);
        for (i, w) in e.pts.0.windows(2).enumerate() {
            let ab = Coord { x: w[1].x - w[0].x, y: w[1].y - w[0].y };
            let l2 = ab.x * ab.x + ab.y * ab.y;
            let t = if l2 < 1e-9 { 0.0 } else { (((p.x - w[0].x) * ab.x + (p.y - w[0].y) * ab.y) / l2).clamp(0.0, 1.0) };
            let q = Coord { x: w[0].x + t * ab.x, y: w[0].y + t * ab.y };
            let d = dist(p, q);
            if d < best.0 {
                best = (d, i, q);
            }
        }
        let (_, seg, q) = best;
        if dist(q, self.nodes[e.a].pos) < 2.0 {
            return e.a;
        }
        if dist(q, self.nodes[e.b].pos) < 2.0 {
            return e.b;
        }
        let n = self.add_node(q);
        let mut first: Vec<Coord<f64>> = e.pts.0[..=seg].to_vec();
        first.push(q);
        let mut second: Vec<Coord<f64>> = vec![q];
        second.extend_from_slice(&e.pts.0[seg + 1..]);
        self.edges[ei] = GEdge { a: e.a, b: n, pts: LineString(first), ..e.clone() };
        self.edges.push(GEdge { a: n, b: e.b, pts: LineString(second), ..e });
        n
    }
}

pub fn build(ctx: &mut Ctx) {
    let mut g = Graph::default();
    let used_xp = build_from_xplane(ctx, &mut g);
    if !used_xp {
        build_from_lines(ctx, &mut g);
    }
    attach_stands(ctx, &mut g);
    classify_nodes(ctx, &mut g);
    emit(ctx, &g);
    ctx.asrn = g;
}

fn build_from_xplane(ctx: &Ctx, g: &mut Graph) -> bool {
    let edges: Vec<_> = ctx.src.route_edges.iter().filter(|e| e.restriction == "runway" || e.restriction.starts_with("taxiway")).collect();
    if edges.is_empty() {
        return false;
    }
    let mut idmap: HashMap<i64, usize> = HashMap::new();
    let pos: HashMap<i64, Coord<f64>> = ctx.src.route_nodes.iter().map(|n| (n.id, ctx.p(n.pos))).collect();
    for e in edges {
        let (Some(pa), Some(pb)) = (pos.get(&e.from), pos.get(&e.to)) else { continue };
        let a = *idmap.entry(e.from).or_insert_with(|| g.add_node(*pa));
        let b = *idmap.entry(e.to).or_insert_with(|| g.add_node(*pb));
        if a == b {
            continue;
        }
        let is_rwy = e.restriction == "runway";
        let active: Vec<String> = e.active_zones.iter().flat_map(|(_, r)| r.iter().cloned()).collect::<BTreeSet<_>>().into_iter().collect();
        let et = if is_rwy { edgetype::RUNWAY } else if !active.is_empty() { edgetype::RUNWAY_EXIT } else { edgetype::TAXIWAY };
        let idrwy = if is_rwy { e.name.clone().map(|n| n.to_uppercase()) } else { None };
        g.edges.push(GEdge {
            a,
            b,
            edgetype: et,
            direc: if e.oneway { direc::FORWARD } else { direc::BIDIRECTIONAL },
            name: if is_rwy { None } else { e.name.clone() },
            wingspan: wingspan_for_restriction(&e.restriction),
            idrwy,
            active,
            pts: LineString(vec![*pa, *pb]),
            source: source::XPLANE,
        });
    }
    true
}

fn key(c: Coord<f64>) -> (i64, i64) {
    ((c.x * 10.0).round() as i64, (c.y * 10.0).round() as i64)
}

/// Fallback graph from OSM taxiway centrelines + runway centrelines.
fn build_from_lines(ctx: &mut Ctx, g: &mut Graph) {
    struct Way {
        pts: Vec<Coord<f64>>,
        name: Option<String>,
        runway: Option<String>,
        width: Option<f64>,
    }
    let mut ways: Vec<Way> = ctx
        .src
        .semantic_lines
        .iter()
        .filter(|l| l.kind == LineKind::TaxiCenterline && l.pts.len() >= 2)
        .map(|l| Way { pts: l.pts.iter().map(|c| ctx.p(*c)).collect(), name: l.name.clone(), runway: None, width: l.width_m })
        .collect();
    for r in &ctx.runways {
        ways.push(Way { pts: vec![r.ends[0], r.ends[1]], name: None, runway: Some(r.idrwy.clone()), width: Some(r.width) });
    }
    if ways.is_empty() {
        ctx.warn("no routing network source (no X-Plane taxi routes, no OSM taxiways)");
        return;
    }
    // Insert crossing points between every pair of ways (runway/taxiway crossings mostly).
    let n = ways.len();
    let mut inserts: Vec<Vec<(usize, f64, Coord<f64>)>> = vec![Vec::new(); n]; // (segment idx, t, point)
    for i in 0..n {
        for j in (i + 1)..n {
            for (si, sa) in ways[i].pts.windows(2).enumerate() {
                for (sj, sb) in ways[j].pts.windows(2).enumerate() {
                    if let Some(p) = ops::seg_intersection(Line::new(sa[0], sa[1]), Line::new(sb[0], sb[1])) {
                        let ta = dist(sa[0], p) / dist(sa[0], sa[1]).max(1e-9);
                        let tb = dist(sb[0], p) / dist(sb[0], sb[1]).max(1e-9);
                        inserts[i].push((si, ta, p));
                        inserts[j].push((sj, tb, p));
                    }
                }
            }
        }
    }
    for (i, ins) in inserts.iter_mut().enumerate() {
        if ins.is_empty() {
            continue;
        }
        ins.sort_by(|a, b| (a.0, a.1).partial_cmp(&(b.0, b.1)).unwrap());
        let old = std::mem::take(&mut ways[i].pts);
        let mut new = Vec::with_capacity(old.len() + ins.len());
        let mut k = 0;
        for (si, seg) in old.windows(2).enumerate() {
            new.push(seg[0]);
            while k < ins.len() && ins[k].0 == si {
                if dist(ins[k].2, seg[0]) > 0.05 && dist(ins[k].2, seg[1]) > 0.05 {
                    new.push(ins[k].2);
                }
                k += 1;
            }
        }
        new.push(*old.last().unwrap());
        ways[i].pts = new;
    }
    // Junctions: endpoints and any coordinate used by >1 way (or twice in one way).
    let mut count: HashMap<(i64, i64), usize> = HashMap::new();
    for w in &ways {
        for (i, p) in w.pts.iter().enumerate() {
            let c = count.entry(key(*p)).or_default();
            *c += if i == 0 || i == w.pts.len() - 1 { 2 } else { 1 };
        }
    }
    let mut node_of: HashMap<(i64, i64), usize> = HashMap::new();
    for w in &ways {
        let mut cur: Vec<Coord<f64>> = vec![w.pts[0]];
        for (i, p) in w.pts.iter().enumerate().skip(1) {
            cur.push(*p);
            let is_junction = count.get(&key(*p)).copied().unwrap_or(0) >= 2 || i == w.pts.len() - 1;
            if is_junction {
                let a = *node_of.entry(key(cur[0])).or_insert_with(|| g.add_node(cur[0]));
                let b = *node_of.entry(key(*p)).or_insert_with(|| g.add_node(*p));
                if a != b && ops::length(&LineString(cur.clone())) > 0.5 {
                    let is_rwy = w.runway.is_some();
                    g.edges.push(GEdge {
                        a,
                        b,
                        edgetype: if is_rwy { edgetype::RUNWAY } else { edgetype::TAXIWAY },
                        direc: direc::BIDIRECTIONAL,
                        name: w.name.clone(),
                        wingspan: w.width.map(|wd| ((wd - 2.0) * 2.0).clamp(15.0, 80.0)).map(|ws| if ws >= 65.0 { 80.0 } else if ws >= 52.0 { 65.0 } else if ws >= 36.0 { 52.0 } else if ws >= 24.0 { 36.0 } else { 24.0 }),
                        idrwy: w.runway.clone(),
                        active: vec![],
                        pts: LineString(std::mem::take(&mut cur)),
                        source: source::OSM,
                    });
                }
                cur = vec![*p];
            }
        }
    }
}

fn attach_stands(ctx: &mut Ctx, g: &mut Graph) {
    if g.edges.is_empty() {
        return;
    }
    let lead_ins: Vec<(String, LineString<f64>)> = ctx.layer(Layer::StandGuidanceLine).iter().filter_map(|f| if let geo_types::Geometry::LineString(l) = &f.geom { Some((f.get_str("idstd")?.to_string(), l.clone())) } else { None }).collect();
    let stands = ctx.stands_local.clone();
    for s in stands {
        // Prefer the far end of the stand's lead-in line as the attachment point.
        let lead = lead_ins.iter().filter(|(id, _)| *id == s.name).map(|(_, l)| l.clone()).max_by(|a, b| ops::length(a).partial_cmp(&ops::length(b)).unwrap());
        let attach = lead.as_ref().map(|l| {
            let f = l.0[0];
            let e = *l.0.last().unwrap();
            if dist(f, s.pos) < dist(e, s.pos) { e } else { f }
        }).unwrap_or(s.pos);
        // Nearest taxiway (non-runway) edge.
        let mut best: Option<(f64, usize)> = None;
        for (i, e) in g.edges.iter().enumerate() {
            if e.edgetype == edgetype::RUNWAY || e.edgetype == edgetype::STAND {
                continue;
            }
            let d = ops::point_line_dist(attach, &e.pts);
            if d <= 250.0 && best.map_or(true, |(bd, _)| d < bd) {
                best = Some((d, i));
            }
        }
        let Some((_, ei)) = best else { continue };
        let pn = match g.find_node_near(attach, 3.0) {
            Some(n) => n,
            None => g.split_edge_at(ei, attach),
        };
        if g.nodes[pn].nodetype == nodetype::TAXIWAY {
            g.nodes[pn].nodetype = nodetype::PARKING;
        }
        let sn = g.add_node(s.pos);
        g.nodes[sn].nodetype = nodetype::STAND;
        g.nodes[sn].idstd = Some(s.name.clone());
        let pts = match &lead {
            Some(l) if dist(*l.0.last().unwrap(), s.pos) <= dist(l.0[0], s.pos) => {
                let mut v = vec![g.nodes[pn].pos];
                v.extend(l.0.iter().copied());
                v.push(s.pos);
                v
            }
            Some(l) => {
                let mut v = vec![g.nodes[pn].pos];
                v.extend(l.0.iter().rev().copied());
                v.push(s.pos);
                v
            }
            None => vec![g.nodes[pn].pos, s.pos],
        };
        let pts: Vec<Coord<f64>> = {
            let mut v = pts;
            v.dedup_by(|a, b| dist(*a, *b) < 0.05);
            v
        };
        if pts.len() < 2 || ops::length(&LineString(pts.clone())) < 0.5 {
            // Stand sits on the taxi route itself: no separate stand edge needed.
            g.nodes[pn].nodetype = nodetype::STAND;
            g.nodes[pn].idstd = Some(s.name.clone());
            g.nodes.pop();
            continue;
        }
        g.edges.push(GEdge { a: pn, b: sn, edgetype: edgetype::STAND, direc: direc::BIDIRECTIONAL, name: None, wingspan: Some(s.wingspan), idrwy: None, active: vec![], pts: LineString(pts), source: source::DERIVED });
    }
}

fn classify_nodes(_ctx: &Ctx, g: &mut Graph) {
    let mut incident: Vec<Vec<usize>> = vec![Vec::new(); g.nodes.len()];
    for (i, e) in g.edges.iter().enumerate() {
        incident[e.a].push(i);
        incident[e.b].push(i);
    }
    for (ni, n) in g.nodes.iter_mut().enumerate() {
        if matches!(n.nodetype, nodetype::STAND | nodetype::PARKING) {
            continue;
        }
        let types: BTreeSet<i64> = incident[ni].iter().map(|&e| g.edges[e].edgetype).collect();
        let rw = types.contains(&edgetype::RUNWAY);
        let ex = types.contains(&edgetype::RUNWAY_EXIT);
        let tw = types.contains(&edgetype::TAXIWAY);
        n.nodetype = if rw && (ex || tw) {
            nodetype::RUNWAY_EXIT
        } else if rw {
            nodetype::RUNWAY
        } else if ex && tw {
            nodetype::HOLDING_POSITION
        } else {
            nodetype::TAXIWAY
        };
        if rw {
            n.idrwy = incident[ni].iter().filter_map(|&e| g.edges[e].idrwy.clone()).next();
        }
    }
}

fn emit(ctx: &mut Ctx, g: &Graph) {
    let icao = ctx.icao.clone();
    let mut names: Vec<BTreeSet<String>> = vec![BTreeSet::new(); g.nodes.len()];
    for e in &g.edges {
        if let Some(n) = &e.name {
            names[e.a].insert(n.clone());
            names[e.b].insert(n.clone());
        }
    }
    for (i, n) in g.nodes.iter().enumerate() {
        let name = if names[i].is_empty() { None } else { Some(names[i].iter().cloned().collect::<Vec<_>>().join("/")) };
        ctx.push(
            AmdbFeature::new(Layer::AsrnNode, Point(n.pos))
                .with("idnetwrk", icao.clone())
                .with("nodeid", i as i64)
                .with("nodetype", n.nodetype)
                .with("name", opt(name))
                .with("idrwy", opt(n.idrwy.clone()))
                .with("idstd", opt(n.idstd.clone()))
                .with("termref", serde_json::Value::Null)
                .with("source", if n.nodetype == nodetype::STAND || n.nodetype == nodetype::PARKING { source::DERIVED } else if g.edges.iter().any(|e| (e.a == i || e.b == i) && e.source == source::XPLANE) { source::XPLANE } else { source::OSM }),
        );
    }
    for (i, e) in g.edges.iter().enumerate() {
        ctx.push(
            AmdbFeature::new(Layer::AsrnEdge, e.pts.clone())
                .with("idnetwrk", icao.clone())
                .with("edgeid", i as i64)
                .with("stnode", e.a as i64)
                .with("ennode", e.b as i64)
                .with("edgetype", e.edgetype)
                .with("direc", e.direc)
                .with("idlin", opt(e.name.clone()))
                .with("idrwy", opt(e.idrwy.clone()))
                .with("idstd", opt(if e.edgetype == edgetype::STAND { g.nodes[e.b].idstd.clone() } else { None }))
                .with("wingspan", opt(e.wingspan))
                .with("edgelen", super::runway::round1(ops::length(&e.pts)))
                .with("actzone", if e.active.is_empty() { serde_json::Value::Null } else { e.active.join(",").into() })
                .with("source", e.source),
        );
    }
}
