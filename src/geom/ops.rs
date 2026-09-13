//! Planar helpers in the local metre frame: lengths, buffers, rectangles, boolean ops.

use geo::{unary_union, Area, BooleanOps, Buffer, Centroid, Contains, Intersects, Orient, Simplify};
use geo::orient::Direction;
use geo_types::{Coord, Line, LineString, MultiLineString, MultiPolygon, Point, Polygon};

pub fn dist(a: Coord<f64>, b: Coord<f64>) -> f64 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

pub fn length(ls: &LineString<f64>) -> f64 {
    ls.0.windows(2).map(|w| dist(w[0], w[1])).sum()
}

/// Heading (degrees clockwise from north) of the vector a -> b.
pub fn heading_deg(a: Coord<f64>, b: Coord<f64>) -> f64 {
    let h = (b.x - a.x).atan2(b.y - a.y).to_degrees();
    (h + 360.0) % 360.0
}

/// Unit vector for a compass heading (degrees clockwise from north).
pub fn unit_from_heading(h_deg: f64) -> Coord<f64> {
    let r = h_deg.to_radians();
    Coord { x: r.sin(), y: r.cos() }
}

pub fn add(a: Coord<f64>, b: Coord<f64>) -> Coord<f64> {
    Coord { x: a.x + b.x, y: a.y + b.y }
}

pub fn scale(a: Coord<f64>, s: f64) -> Coord<f64> {
    Coord { x: a.x * s, y: a.y * s }
}

/// Perpendicular (rotated 90 degrees clockwise) of a unit vector: right-hand side.
pub fn right_of(u: Coord<f64>) -> Coord<f64> {
    Coord { x: u.y, y: -u.x }
}

pub fn point_seg_dist(p: Coord<f64>, a: Coord<f64>, b: Coord<f64>) -> f64 {
    let ab = Coord { x: b.x - a.x, y: b.y - a.y };
    let l2 = ab.x * ab.x + ab.y * ab.y;
    if l2 < 1e-12 {
        return dist(p, a);
    }
    let t = (((p.x - a.x) * ab.x + (p.y - a.y) * ab.y) / l2).clamp(0.0, 1.0);
    dist(p, Coord { x: a.x + t * ab.x, y: a.y + t * ab.y })
}

pub fn point_line_dist(p: Coord<f64>, ls: &LineString<f64>) -> f64 {
    ls.0.windows(2).map(|w| point_seg_dist(p, w[0], w[1])).fold(f64::INFINITY, f64::min)
}

/// Axis-aligned rectangle helper: a rectangle of `length` along heading `h` starting at
/// `start`, with `width` centred on the axis.
pub fn rect_along(start: Coord<f64>, h_deg: f64, length: f64, width: f64) -> Polygon<f64> {
    let u = unit_from_heading(h_deg);
    let r = scale(right_of(u), width / 2.0);
    let end = add(start, scale(u, length));
    let ring = vec![
        add(start, scale(r, -1.0)),
        add(start, r),
        add(end, r),
        add(end, scale(r, -1.0)),
        add(start, scale(r, -1.0)),
    ];
    Polygon::new(LineString(ring), vec![]).orient(Direction::Default)
}

/// Rectangle between two centreline points with a width.
pub fn rect_between(a: Coord<f64>, b: Coord<f64>, width: f64) -> Polygon<f64> {
    rect_along(a, heading_deg(a, b), dist(a, b), width)
}

/// Rectangle centred at `c` with heading `h` (length along heading).
pub fn rect_centered(c: Coord<f64>, h_deg: f64, length: f64, width: f64) -> Polygon<f64> {
    let u = unit_from_heading(h_deg);
    rect_along(add(c, scale(u, -length / 2.0)), h_deg, length, width)
}

pub fn circle(c: Coord<f64>, r: f64, n: usize) -> Polygon<f64> {
    let n = n.max(8);
    let mut ring: Vec<Coord<f64>> = (0..n)
        .map(|i| {
            let t = i as f64 / n as f64 * std::f64::consts::TAU;
            Coord { x: c.x + r * t.cos(), y: c.y + r * t.sin() }
        })
        .collect();
    ring.push(ring[0]);
    Polygon::new(LineString(ring), vec![])
}

/// Union any number of polygons into one MultiPolygon (empty input -> empty).
pub fn union_all(polys: &[Polygon<f64>]) -> MultiPolygon<f64> {
    let valid: Vec<&Polygon<f64>> = polys.iter().filter(|p| p.exterior().0.len() >= 4 && p.unsigned_area() > 1e-6).collect();
    if valid.is_empty() {
        return MultiPolygon(vec![]);
    }
    unary_union(valid.into_iter())
}

/// Buffer a polyline by `w` metres with round joins/caps.
pub fn buffer_line(ls: &LineString<f64>, w: f64) -> MultiPolygon<f64> {
    if ls.0.len() < 2 || w <= 0.0 {
        return MultiPolygon(vec![]);
    }
    ls.buffer(w)
}

/// Outward buffer of a polygon (holes are kept if they survive).
pub fn buffer_polygon(p: &Polygon<f64>, w: f64) -> MultiPolygon<f64> {
    if w <= 0.0 {
        return MultiPolygon(vec![p.clone()]);
    }
    p.buffer(w)
}

pub fn buffer_multi(mp: &MultiPolygon<f64>, w: f64) -> MultiPolygon<f64> {
    if mp.0.is_empty() {
        return MultiPolygon(vec![]);
    }
    mp.buffer(w)
}

pub fn mp_union(a: &MultiPolygon<f64>, b: &MultiPolygon<f64>) -> MultiPolygon<f64> {
    if a.0.is_empty() {
        return b.clone();
    }
    if b.0.is_empty() {
        return a.clone();
    }
    a.union(b)
}

pub fn mp_difference(a: &MultiPolygon<f64>, b: &MultiPolygon<f64>) -> MultiPolygon<f64> {
    if a.0.is_empty() || b.0.is_empty() {
        return a.clone();
    }
    a.difference(b)
}

pub fn mp_intersection(a: &MultiPolygon<f64>, b: &MultiPolygon<f64>) -> MultiPolygon<f64> {
    if a.0.is_empty() || b.0.is_empty() {
        return MultiPolygon(vec![]);
    }
    a.intersection(b)
}

/// Portions of `ls` inside (`inside = true`) or outside the polygon set.
pub fn clip_line(mp: &MultiPolygon<f64>, ls: &LineString<f64>, inside: bool) -> Vec<LineString<f64>> {
    if mp.0.is_empty() {
        return if inside { vec![] } else { vec![ls.clone()] };
    }
    let out = mp.clip(&MultiLineString(vec![ls.clone()]), !inside);
    out.0.into_iter().filter(|l| l.0.len() >= 2 && length(l) > 0.01).collect()
}

pub fn mp_contains_point(mp: &MultiPolygon<f64>, c: Coord<f64>) -> bool {
    mp.0.iter().any(|p| p.contains(&Point(c)))
}

pub fn poly_contains_point(p: &Polygon<f64>, c: Coord<f64>) -> bool {
    p.contains(&Point(c))
}

pub fn intersects_line(mp: &MultiPolygon<f64>, ls: &LineString<f64>) -> bool {
    mp.0.iter().any(|p| p.intersects(ls))
}

pub fn centroid(p: &Polygon<f64>) -> Coord<f64> {
    p.centroid().map(|c| c.0).unwrap_or_else(|| p.exterior().0[0])
}

pub fn mp_centroid(mp: &MultiPolygon<f64>) -> Option<Coord<f64>> {
    mp.centroid().map(|c| c.0)
}

/// Drop collinear/near-duplicate vertices and enforce DO-272 ring orientation
/// (exterior counter-clockwise, holes clockwise).
pub fn tidy_polygon(p: &Polygon<f64>) -> Option<Polygon<f64>> {
    let s = p.simplify(0.1).orient(Direction::Default);
    if s.exterior().0.len() < 4 || s.unsigned_area() < 0.5 {
        return None;
    }
    Some(s)
}

pub fn tidy_multi(mp: &MultiPolygon<f64>) -> Vec<Polygon<f64>> {
    mp.0.iter().filter_map(tidy_polygon).collect()
}

/// Close a ring if needed and remove consecutive duplicates.
pub fn close_ring(mut pts: Vec<Coord<f64>>) -> LineString<f64> {
    pts.dedup_by(|a, b| dist(*a, *b) < 1e-6);
    if pts.len() >= 2 && dist(pts[0], *pts.last().unwrap()) > 1e-6 {
        pts.push(pts[0]);
    }
    LineString(pts)
}

/// Split a linestring where it crosses the boundary of `mp`; returns (inside, outside) parts.
pub fn split_by(mp: &MultiPolygon<f64>, ls: &LineString<f64>) -> (Vec<LineString<f64>>, Vec<LineString<f64>>) {
    (clip_line(mp, ls, true), clip_line(mp, ls, false))
}

/// Segment-segment intersection point, if any.
pub fn seg_intersection(a: Line<f64>, b: Line<f64>) -> Option<Coord<f64>> {
    use geo::line_intersection::{line_intersection, LineIntersection};
    match line_intersection(a, b) {
        Some(LineIntersection::SinglePoint { intersection, .. }) => Some(intersection),
        _ => None,
    }
}

/// Point along a polyline at distance `d` from its start (clamped).
pub fn point_at(ls: &LineString<f64>, d: f64) -> Coord<f64> {
    let mut acc = 0.0;
    for w in ls.0.windows(2) {
        let l = dist(w[0], w[1]);
        if acc + l >= d && l > 0.0 {
            let t = (d - acc) / l;
            return Coord { x: w[0].x + t * (w[1].x - w[0].x), y: w[0].y + t * (w[1].y - w[0].y) };
        }
        acc += l;
    }
    *ls.0.last().unwrap()
}

/// Convex hull of a point cloud as a polygon (None if fewer than 3 points).
pub fn hull(pts: &[Coord<f64>]) -> Option<Polygon<f64>> {
    use geo::ConvexHull;
    if pts.len() < 3 {
        return None;
    }
    let mp = geo_types::MultiPoint(pts.iter().map(|c| Point(*c)).collect());
    let h = mp.convex_hull();
    if h.exterior().0.len() < 4 {
        None
    } else {
        Some(h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_between_has_expected_area() {
        let p = rect_between(Coord { x: 0.0, y: 0.0 }, Coord { x: 100.0, y: 0.0 }, 20.0);
        assert!((p.unsigned_area() - 2000.0).abs() < 1e-6);
    }

    #[test]
    fn buffer_line_area_is_reasonable() {
        let ls = LineString(vec![Coord { x: 0.0, y: 0.0 }, Coord { x: 100.0, y: 0.0 }]);
        let b = buffer_line(&ls, 5.0);
        let a = b.unsigned_area();
        // rectangle 100x10 + two half discs r=5 (polygonal approximation)
        assert!(a > 1000.0 + 60.0 && a < 1000.0 + 80.0, "{a}");
    }

    #[test]
    fn clip_keeps_inside_part() {
        let sq = MultiPolygon(vec![rect_between(Coord { x: 0.0, y: 0.0 }, Coord { x: 10.0, y: 0.0 }, 10.0)]);
        let ls = LineString(vec![Coord { x: -5.0, y: 0.0 }, Coord { x: 15.0, y: 0.0 }]);
        let inside = clip_line(&sq, &ls, true);
        assert_eq!(inside.len(), 1);
        assert!((length(&inside[0]) - 10.0).abs() < 1e-6);
        let outside = clip_line(&sq, &ls, false);
        assert_eq!(outside.len(), 2);
    }

    #[test]
    fn heading_and_unit_agree() {
        let u = unit_from_heading(90.0);
        assert!((u.x - 1.0).abs() < 1e-9 && u.y.abs() < 1e-9);
        assert!((heading_deg(Coord { x: 0.0, y: 0.0 }, Coord { x: 0.0, y: 5.0 })).abs() < 1e-9);
    }
}
