use geo_types::Coord;

/// apt.dat nodes may carry one Bézier control point that shapes both the incoming
/// and outgoing curve (mirrored about the node). This tessellates the segment from
/// `p0` to `p1` where `c0` is the control of `p0` and `c1` the control of `p1`,
/// using the cubic construction X-Plane itself applies.
pub fn tessellate(p0: Coord<f64>, c0: Option<Coord<f64>>, p1: Coord<f64>, c1: Option<Coord<f64>>, steps: usize) -> Vec<Coord<f64>> {
    let h0 = c0.unwrap_or(p0);
    let h1 = c1.map(|c| Coord { x: 2.0 * p1.x - c.x, y: 2.0 * p1.y - c.y }).unwrap_or(p1);
    if c0.is_none() && c1.is_none() {
        return vec![p1];
    }
    let n = steps.max(2);
    (1..=n)
        .map(|i| {
            let t = i as f64 / n as f64;
            let mt = 1.0 - t;
            let a = mt * mt * mt;
            let b = 3.0 * mt * mt * t;
            let c = 3.0 * mt * t * t;
            let d = t * t * t;
            Coord { x: a * p0.x + b * h0.x + c * h1.x + d * p1.x, y: a * p0.y + b * h0.y + c * h1.y + d * p1.y }
        })
        .collect()
}

/// Choose a step count from chord length so long curves stay smooth (roughly 1 vertex / 2 m).
pub fn steps_for(p0: Coord<f64>, p1: Coord<f64>) -> usize {
    let d = ((p1.x - p0.x).powi(2) + (p1.y - p0.y).powi(2)).sqrt();
    ((d / 2.0).ceil() as usize).clamp(4, 48)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn straight_segment_yields_endpoint_only() {
        let v = tessellate(Coord { x: 0.0, y: 0.0 }, None, Coord { x: 10.0, y: 0.0 }, None, 8);
        assert_eq!(v, vec![Coord { x: 10.0, y: 0.0 }]);
    }

    #[test]
    fn curve_ends_at_p1_and_bows_toward_control() {
        let v = tessellate(Coord { x: 0.0, y: 0.0 }, Some(Coord { x: 5.0, y: 5.0 }), Coord { x: 10.0, y: 0.0 }, None, 8);
        assert_eq!(v.len(), 8);
        let last = v.last().unwrap();
        assert!((last.x - 10.0).abs() < 1e-9 && last.y.abs() < 1e-9);
        assert!(v[3].y > 0.5);
    }
}
