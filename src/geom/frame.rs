use geo_types::{Coord, Geometry, LineString, MultiLineString, MultiPoint, MultiPolygon, Point, Polygon};

const R: f64 = 6_371_008.8;

/// Azimuthal equidistant projection centred on a reference point.
/// `forward` maps (lon, lat) degrees -> (x east, y north) metres; `inverse` reverses.
#[derive(Debug, Clone, Copy)]
pub struct LocalFrame {
    pub lat0: f64,
    pub lon0: f64,
    sin0: f64,
    cos0: f64,
}

impl LocalFrame {
    pub fn new(lat0: f64, lon0: f64) -> Self {
        let l = lat0.to_radians();
        Self { lat0, lon0, sin0: l.sin(), cos0: l.cos() }
    }

    pub fn forward(&self, lon: f64, lat: f64) -> Coord<f64> {
        let phi = lat.to_radians();
        let dl = (lon - self.lon0).to_radians();
        let (sinp, cosp) = (phi.sin(), phi.cos());
        let cos_c = self.sin0 * sinp + self.cos0 * cosp * dl.cos();
        let c = cos_c.clamp(-1.0, 1.0).acos();
        let k = if c.abs() < 1e-12 { 1.0 } else { c / c.sin() };
        Coord {
            x: R * k * cosp * dl.sin(),
            y: R * k * (self.cos0 * sinp - self.sin0 * cosp * dl.cos()),
        }
    }

    /// Returns (lon, lat) in degrees.
    pub fn inverse(&self, c: Coord<f64>) -> (f64, f64) {
        let rho = (c.x * c.x + c.y * c.y).sqrt();
        if rho < 1e-9 {
            return (self.lon0, self.lat0);
        }
        let cc = rho / R;
        let (sinc, cosc) = (cc.sin(), cc.cos());
        let lat = (cosc * self.sin0 + c.y * sinc * self.cos0 / rho).clamp(-1.0, 1.0).asin();
        let lon = self.lon0.to_radians()
            + (c.x * sinc).atan2(rho * self.cos0 * cosc - c.y * self.sin0 * sinc);
        (lon.to_degrees(), lat.to_degrees())
    }

    pub fn fwd_c(&self, c: Coord<f64>) -> Coord<f64> {
        self.forward(c.x, c.y)
    }

    pub fn fwd_pt(&self, lon: f64, lat: f64) -> Point<f64> {
        Point(self.forward(lon, lat))
    }

    pub fn fwd_line(&self, pts: &[Coord<f64>]) -> LineString<f64> {
        LineString(pts.iter().map(|c| self.forward(c.x, c.y)).collect())
    }

    /// Convert a local-frame geometry back to lon/lat degrees.
    pub fn to_wgs84(&self, g: &Geometry<f64>) -> Geometry<f64> {
        let inv = |c: &Coord<f64>| {
            let (lon, lat) = self.inverse(*c);
            Coord { x: lon, y: lat }
        };
        let ls = |l: &LineString<f64>| LineString(l.0.iter().map(inv).collect());
        let poly = |p: &Polygon<f64>| Polygon::new(ls(p.exterior()), p.interiors().iter().map(ls).collect());
        match g {
            Geometry::Point(p) => Geometry::Point(Point(inv(&p.0))),
            Geometry::LineString(l) => Geometry::LineString(ls(l)),
            Geometry::Polygon(p) => Geometry::Polygon(poly(p)),
            Geometry::MultiPoint(m) => Geometry::MultiPoint(MultiPoint(m.0.iter().map(|p| Point(inv(&p.0))).collect())),
            Geometry::MultiLineString(m) => Geometry::MultiLineString(MultiLineString(m.0.iter().map(ls).collect())),
            Geometry::MultiPolygon(m) => Geometry::MultiPolygon(MultiPolygon(m.0.iter().map(poly).collect())),
            other => other.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_is_exact_at_airport_scale() {
        let f = LocalFrame::new(28.5665, 77.1031);
        for (lon, lat) in [(77.1031, 28.5665), (77.15, 28.60), (77.05, 28.52), (77.20, 28.49)] {
            let c = f.forward(lon, lat);
            let (lon2, lat2) = f.inverse(c);
            assert!((lon - lon2).abs() < 1e-9, "{lon} vs {lon2}");
            assert!((lat - lat2).abs() < 1e-9, "{lat} vs {lat2}");
        }
    }

    #[test]
    fn one_degree_of_latitude_is_about_111km() {
        let f = LocalFrame::new(0.0, 0.0);
        let c = f.forward(0.0, 1.0);
        assert!((c.y - 111_195.0).abs() < 50.0, "{}", c.y);
        assert!(c.x.abs() < 1e-6);
        let e = f.forward(1.0, 0.0);
        assert!((e.x - 111_195.0).abs() < 50.0);
    }
}
