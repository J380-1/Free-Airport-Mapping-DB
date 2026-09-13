//! In-memory OSM element store plus geometry assembly (ways -> lines/rings,
//! multipolygon relations -> outer/inner rings).

use geo_types::Coord;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

pub type Tags = BTreeMap<String, String>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Way {
    pub id: i64,
    pub nodes: Vec<i64>,
    pub tags: Tags,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    pub kind: char, // 'n', 'w', 'r'
    pub id: i64,
    pub role: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Relation {
    pub id: i64,
    pub members: Vec<Member>,
    pub tags: Tags,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Store {
    pub nodes: HashMap<i64, Coord<f64>>,
    pub node_tags: HashMap<i64, Tags>,
    pub ways: HashMap<i64, Way>,
    pub relations: HashMap<i64, Relation>,
}

/// An assembled polygon: outer ring + holes, all closed, in lon/lat.
#[derive(Debug, Clone)]
pub struct PolyGeom {
    pub outer: Vec<Coord<f64>>,
    pub holes: Vec<Vec<Coord<f64>>>,
}

impl Store {
    pub fn way_coords(&self, w: &Way) -> Vec<Coord<f64>> {
        w.nodes.iter().filter_map(|n| self.nodes.get(n).copied()).collect()
    }

    pub fn way_is_closed(&self, w: &Way) -> bool {
        w.nodes.len() >= 4 && w.nodes.first() == w.nodes.last()
    }

    /// True when the way should be treated as an area (closed and tagged as such).
    pub fn way_is_area(&self, w: &Way) -> bool {
        if !self.way_is_closed(w) {
            return false;
        }
        if let Some(a) = w.tags.get("area") {
            return a != "no";
        }
        // Closed aeroway=runway/taxiway ways are lines unless area=yes; everything
        // else closed (building, apron, natural, landuse, ...) is an area.
        match w.tags.get("aeroway").map(String::as_str) {
            Some("runway") | Some("taxiway") | Some("taxilane") => false,
            _ => !w.tags.contains_key("barrier") && !w.tags.contains_key("highway") && !w.tags.contains_key("power"),
        }
    }

    /// Polygon(s) for a multipolygon/boundary relation, or None if it is not one.
    pub fn relation_polygons(&self, r: &Relation) -> Vec<PolyGeom> {
        let mut outers: Vec<Vec<Coord<f64>>> = Vec::new();
        let mut inners: Vec<Vec<Coord<f64>>> = Vec::new();
        let mut outer_ways = Vec::new();
        let mut inner_ways = Vec::new();
        for m in &r.members {
            if m.kind != 'w' {
                continue;
            }
            let Some(w) = self.ways.get(&m.id) else { continue };
            let coords = self.way_coords(w);
            if coords.len() < 2 {
                continue;
            }
            if m.role == "inner" {
                inner_ways.push(coords);
            } else {
                outer_ways.push(coords);
            }
        }
        outers.extend(join_rings(outer_ways));
        inners.extend(join_rings(inner_ways));
        // Assign holes to the outer that contains them (by first vertex).
        let mut polys: Vec<PolyGeom> = outers.into_iter().map(|o| PolyGeom { outer: o, holes: vec![] }).collect();
        for h in inners {
            let p0 = h[0];
            if let Some(p) = polys.iter_mut().find(|p| point_in_ring(p0, &p.outer)) {
                p.holes.push(h);
            }
        }
        polys
    }
}

/// Join open way fragments into closed rings by matching endpoints.
pub fn join_rings(mut frags: Vec<Vec<Coord<f64>>>) -> Vec<Vec<Coord<f64>>> {
    let mut rings = Vec::new();
    let same = |a: Coord<f64>, b: Coord<f64>| (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9;
    while let Some(mut cur) = frags.pop() {
        let mut progressed = true;
        while progressed && !same(cur[0], *cur.last().unwrap()) {
            progressed = false;
            let end = *cur.last().unwrap();
            if let Some(i) = frags.iter().position(|f| same(f[0], end) || same(*f.last().unwrap(), end)) {
                let mut f = frags.remove(i);
                if same(*f.last().unwrap(), end) {
                    f.reverse();
                }
                cur.extend(f.into_iter().skip(1));
                progressed = true;
            }
        }
        if cur.len() >= 4 && same(cur[0], *cur.last().unwrap()) {
            rings.push(cur);
        } else if cur.len() >= 3 {
            cur.push(cur[0]);
            rings.push(cur);
        }
    }
    rings
}

pub fn point_in_ring(p: Coord<f64>, ring: &[Coord<f64>]) -> bool {
    let mut inside = false;
    let n = ring.len();
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (ring[i], ring[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(x: f64, y: f64) -> Coord<f64> {
        Coord { x, y }
    }

    #[test]
    fn joins_fragments_into_ring() {
        let frags = vec![vec![c(0.0, 0.0), c(1.0, 0.0)], vec![c(1.0, 1.0), c(1.0, 0.0)], vec![c(1.0, 1.0), c(0.0, 1.0), c(0.0, 0.0)]];
        let rings = join_rings(frags);
        assert_eq!(rings.len(), 1);
        assert_eq!(rings[0].len(), 5);
        assert!(point_in_ring(c(0.5, 0.5), &rings[0]));
        assert!(!point_in_ring(c(1.5, 0.5), &rings[0]));
    }

    #[test]
    fn multipolygon_assigns_holes() {
        let mut s = Store::default();
        let pts = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0), (4.0, 4.0), (6.0, 4.0), (6.0, 6.0), (4.0, 6.0)];
        for (i, (x, y)) in pts.iter().enumerate() {
            s.nodes.insert(i as i64, c(*x, *y));
        }
        s.ways.insert(1, Way { id: 1, nodes: vec![0, 1, 2, 3, 0], tags: Tags::new() });
        s.ways.insert(2, Way { id: 2, nodes: vec![4, 5, 6, 7, 4], tags: Tags::new() });
        let r = Relation {
            id: 9,
            members: vec![Member { kind: 'w', id: 1, role: "outer".into() }, Member { kind: 'w', id: 2, role: "inner".into() }],
            tags: Tags::new(),
        };
        let polys = s.relation_polygons(&r);
        assert_eq!(polys.len(), 1);
        assert_eq!(polys[0].holes.len(), 1);
    }
}
