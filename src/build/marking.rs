//! Runway markings generated from the marking code using ICAO Annex 14 dimensions:
//! threshold stripes, designation box, centreline stripes, aiming point, touchdown
//! zone stripes, side stripes, displaced-threshold arrows and blastpad chevrons.

use super::{Ctx, RwyGeom};
use crate::geom::ops::{self, add, scale, unit_from_heading};
use crate::model::codes::{marktype, rwymktyp, source};
use crate::model::{AmdbFeature, Layer};
use geo_types::{Coord, LineString, Polygon};

struct Axis {
    origin: Coord<f64>, // threshold
    u: Coord<f64>,      // along runway (landing direction)
    r: Coord<f64>,      // to the right
}

impl Axis {
    /// Rectangle from `along` (start) for `len`, centred at lateral offset `lat`, width `w`.
    fn rect(&self, along: f64, len: f64, lat: f64, w: f64) -> Polygon<f64> {
        let c = add(add(self.origin, scale(self.u, along)), scale(self.r, lat));
        let p = |a: f64, l: f64| add(add(c, scale(self.u, a)), scale(self.r, l));
        Polygon::new(LineString(vec![p(0.0, -w / 2.0), p(0.0, w / 2.0), p(len, w / 2.0), p(len, -w / 2.0), p(0.0, -w / 2.0)]), vec![])
    }
}

fn threshold_stripe_count(width: f64) -> usize {
    if width < 20.0 { 4 } else if width < 26.0 { 6 } else if width < 40.0 { 8 } else if width < 55.0 { 12 } else { 16 }
}

fn aiming_point(lda: f64) -> (f64, f64, f64) {
    // (distance from threshold, stripe length, stripe width)
    if lda < 800.0 { (150.0, 30.0, 4.0) } else if lda < 1200.0 { (250.0, 30.0, 6.0) } else if lda < 2400.0 { (300.0, 45.0, 6.0) } else { (400.0, 60.0, 10.0) }
}

fn tdz_pairs(lda: f64) -> usize {
    if lda < 900.0 { 1 } else if lda < 1200.0 { 2 } else if lda < 1500.0 { 3 } else if lda < 2400.0 { 4 } else { 6 }
}

pub fn build(ctx: &mut Ctx) {
    let runways: Vec<RwyGeom> = ctx.runways.clone();
    for g in &runways {
        let r = &ctx.src.runways[g.src_index];
        for k in 0..2 {
            let end = &r.ends[k];
            if end.marking == rwymktyp::NONE || end.marking == rwymktyp::UNKNOWN {
                continue;
            }
            let dir = if k == 0 { g.heading } else { (g.heading + 180.0) % 360.0 };
            let u = unit_from_heading(dir);
            let ax = Axis { origin: g.thresholds[k], u, r: ops::right_of(u) };
            let lda = ops::dist(g.thresholds[k], g.thresholds[1 - k]);
            let half = lda / 2.0; // each end marks its own half
            let push = |ctx: &mut Ctx, poly: Polygon<f64>, mt: i64, text: Option<String>| {
                let mut f = AmdbFeature::new(Layer::RunwayMarking, poly).with("idrwy", g.idrwy.clone()).with("idthr", end.ident.clone()).with("marktype", mt).with("rwymktyp", end.marking).with("source", source::DERIVED);
                if let Some(t) = text {
                    f.set("text", t);
                }
                ctx.push(f);
            };
            let designation_start;
            // Threshold stripes (instrument runways).
            if end.marking >= rwymktyp::NON_PRECISION {
                let n = threshold_stripe_count(g.width);
                let usable = g.width - 6.0;
                let pitch = usable / n as f64;
                for i in 0..n {
                    let lat = -usable / 2.0 + pitch * (i as f64 + 0.5);
                    push(ctx, ax.rect(6.0, 30.0, lat, 1.8), marktype::THRESHOLD, None);
                }
                designation_start = 48.0;
            } else {
                designation_start = 12.0;
            }
            // Designation box (numerals are drawn by the client from `text`).
            let des_w = (g.width * 0.5).clamp(6.0, 14.0);
            push(ctx, ax.rect(designation_start, 9.0, 0.0, des_w), marktype::DESIGNATION, Some(end.ident.clone()));
            // Centreline stripes: 30 m stripes, 20 m gaps, to the runway midpoint.
            let mut along = designation_start + 9.0 + 12.0;
            while along + 30.0 <= half {
                push(ctx, ax.rect(along, 30.0, 0.0, 0.9), marktype::CENTERLINE, None);
                along += 50.0;
            }
            if end.marking >= rwymktyp::NON_PRECISION {
                let (ap_d, ap_len, ap_w) = aiming_point(lda);
                let inner = if g.width >= 45.0 { 22.5 } else { 18.0 };
                if ap_d + ap_len < half {
                    for s in [-1.0, 1.0] {
                        push(ctx, ax.rect(ap_d, ap_len, s * (inner / 2.0 + ap_w / 2.0), ap_w), marktype::AIMING_POINT, None);
                    }
                }
            }
            if end.marking >= rwymktyp::PRECISION {
                // Touchdown zone stripes at 150 m intervals; the aiming point occupies one slot.
                let (ap_d, _, _) = aiming_point(lda);
                let pairs = tdz_pairs(lda);
                let counts: [usize; 6] = match pairs { 6 => [3, 3, 2, 2, 1, 1], 4 => [3, 2, 2, 1, 0, 0], 3 => [3, 2, 1, 0, 0, 0], 2 => [2, 1, 0, 0, 0, 0], _ => [1, 0, 0, 0, 0, 0] };
                let inner = if g.width >= 45.0 { 22.5 } else { 18.0 };
                for i in 0..6 {
                    let d = 150.0 * (i as f64 + 1.0);
                    if counts[i] == 0 || (d - ap_d).abs() < 1.0 || d + 22.5 > half {
                        continue;
                    }
                    for s in [-1.0, 1.0] {
                        for j in 0..counts[i] {
                            let lat = s * (inner / 2.0 + 0.9 + j as f64 * 1.5);
                            push(ctx, ax.rect(d, 22.5, lat, 1.8), marktype::TOUCHDOWN_ZONE, None);
                        }
                    }
                }
                // Side stripes for this half.
                for s in [-1.0, 1.0] {
                    push(ctx, ax.rect(0.0, half, s * (g.width / 2.0 - 0.45), 0.9), marktype::SIDE_STRIPE, None);
                }
            }
            // Displaced threshold arrows (pointing at the threshold).
            let displaced = ops::dist(g.ends[k], g.thresholds[k]);
            if displaced > 30.0 {
                let mut d = -displaced + 15.0;
                while d < -10.0 {
                    push(ctx, ax.rect(d, 20.0, 0.0, 0.9), marktype::DISPLACED_ARROW, None);
                    d += 30.0;
                }
            }
            // Blastpad chevrons.
            if end.blastpad_m > 20.0 {
                let mut d = -displaced - 10.0;
                let stop = -displaced - end.blastpad_m;
                while d - 15.0 > stop {
                    for s in [-1.0, 1.0] {
                        let a = add(add(ax.origin, scale(ax.u, d)), scale(ax.r, 0.0));
                        let b = add(add(ax.origin, scale(ax.u, d - 15.0)), scale(ax.r, s * (g.width / 2.0 - 1.0)));
                        push(ctx, ops::rect_between(a, b, 0.9), marktype::CHEVRON, None);
                    }
                    d -= 30.0;
                }
            }
        }
    }
}
