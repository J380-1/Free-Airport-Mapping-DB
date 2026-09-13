//! Convert IR rings (with Bézier controls and per-vertex codes) into local-frame geometry.

use crate::geom::bezier;
use crate::geom::ops;
use crate::geom::LocalFrame;
use crate::ir::{Ring, Vertex};
use geo_types::{Coord, LineString, Polygon};

/// A run of consecutive segments sharing the same line/light codes.
#[derive(Debug, Clone)]
pub struct CodedRun {
    pub line: u16,
    pub light: u16,
    pub pts: LineString<f64>,
}

fn seg_points(frame: &LocalFrame, a: &Vertex, b: &Vertex) -> Vec<Coord<f64>> {
    let p0 = frame.fwd_c(a.pos);
    let p1 = frame.fwd_c(b.pos);
    let c0 = a.ctrl.map(|c| frame.fwd_c(c));
    let c1 = b.ctrl.map(|c| frame.fwd_c(c));
    bezier::tessellate(p0, c0, p1, c1, bezier::steps_for(p0, p1))
}

/// Full outline of a ring as one linestring (closed rings repeat the first point).
pub fn ring_to_linestring(frame: &LocalFrame, ring: &Ring) -> LineString<f64> {
    let n = ring.verts.len();
    if n == 0 {
        return LineString(vec![]);
    }
    let mut pts = vec![frame.fwd_c(ring.verts[0].pos)];
    let segs = if ring.closed { n } else { n - 1 };
    for i in 0..segs {
        let a = &ring.verts[i];
        let b = &ring.verts[(i + 1) % n];
        pts.extend(seg_points(frame, a, b));
    }
    if ring.closed {
        ops::close_ring(pts)
    } else {
        pts.dedup_by(|a, b| ops::dist(*a, *b) < 1e-6);
        LineString(pts)
    }
}

pub fn ring_to_polygon(frame: &LocalFrame, outer: &Ring, holes: &[Ring]) -> Option<Polygon<f64>> {
    let ext = ring_to_linestring(frame, outer);
    if ext.0.len() < 4 {
        return None;
    }
    let ints: Vec<LineString<f64>> = holes.iter().map(|h| ring_to_linestring(frame, h)).filter(|l| l.0.len() >= 4).collect();
    ops::tidy_polygon(&Polygon::new(ext, ints))
}

/// Split a ring into runs of equal (line, light) code. Segments with code 0 are dropped.
pub fn coded_runs(frame: &LocalFrame, ring: &Ring) -> Vec<CodedRun> {
    let n = ring.verts.len();
    let mut out: Vec<CodedRun> = Vec::new();
    if n < 2 {
        return out;
    }
    let segs = if ring.closed { n } else { n - 1 };
    let mut cur: Option<CodedRun> = None;
    for i in 0..segs {
        let a = &ring.verts[i];
        let b = &ring.verts[(i + 1) % n];
        let (line, light) = (a.line, a.light);
        if line == 0 && light == 0 {
            if let Some(c) = cur.take() {
                out.push(c);
            }
            continue;
        }
        let pts = seg_points(frame, a, b);
        match cur.as_mut() {
            Some(c) if c.line == line && c.light == light => c.pts.0.extend(pts),
            _ => {
                if let Some(c) = cur.take() {
                    out.push(c);
                }
                let mut ls = vec![frame.fwd_c(a.pos)];
                ls.extend(pts);
                cur = Some(CodedRun { line, light, pts: LineString(ls) });
            }
        }
    }
    if let Some(c) = cur.take() {
        out.push(c);
    }
    // Merge a closed ring's last run into its first when they share codes.
    if ring.closed && out.len() > 1 {
        let first_code = (out[0].line, out[0].light);
        let last_code = (out[out.len() - 1].line, out[out.len() - 1].light);
        if first_code == last_code && ring.verts[0].line != 0 && ring.verts[n - 1].line != 0 {
            let mut last = out.pop().unwrap();
            let first = out.remove(0);
            last.pts.0.extend(first.pts.0.into_iter().skip(1));
            out.insert(0, last);
        }
    }
    out.retain(|r| r.pts.0.len() >= 2 && ops::length(&r.pts) > 0.2);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(lat: f64, lon: f64, line: u16) -> Vertex {
        Vertex { pos: Coord { x: lon, y: lat }, ctrl: None, line, light: 0 }
    }

    #[test]
    fn splits_runs_by_code() {
        let frame = LocalFrame::new(0.0, 0.0);
        let ring = Ring { verts: vec![v(0.0, 0.0, 1), v(0.0, 0.001, 1), v(0.0, 0.002, 4), v(0.0, 0.003, 0), v(0.0, 0.004, 1)], closed: false };
        let runs = coded_runs(&frame, &ring);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].line, 1);
        assert_eq!(runs[0].pts.0.len(), 3);
        assert_eq!(runs[1].line, 4);
        let poly_ring = Ring { verts: vec![v(0.0, 0.0, 0), v(0.0, 0.001, 0), v(0.001, 0.001, 0), v(0.001, 0.0, 0)], closed: true };
        let p = ring_to_polygon(&frame, &poly_ring, &[]).unwrap();
        assert_eq!(p.exterior().0.len(), 5);
    }
}
