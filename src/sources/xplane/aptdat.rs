//! Parser for X-Plane `apt.dat` (format 1000-1200) into the intermediate model.

use crate::ir::*;
use crate::model::codes::{lighting, rwymktyp, source, station, surftype};
use anyhow::{bail, Context, Result};
use geo_types::Coord;

/// Parse one airport (the first `1`/`16`/`17` block, or the one matching `want_icao`).
pub fn parse(text: &str, want_icao: Option<&str>) -> Result<SourceAirport> {
    let mut lines = text.lines().map(|l| l.trim_end_matches('\r'));
    // Skip header lines: "I"/"A" then version line.
    let mut cur: Option<SourceAirport> = None;
    let mut collecting = false;
    let mut ring_target: RingTarget = RingTarget::None;
    let mut pending_pavement: Option<Pavement> = None;
    let mut pending_rings: Vec<Ring> = Vec::new();
    let mut open_verts: Vec<Vertex> = Vec::new();
    let mut pending_line_name: Option<String> = None;
    let mut last_stand: Option<usize> = None;
    let mut last_edge: Option<usize> = None;

    let flush_ring = |ring_target: &mut RingTarget,
                          pending_pavement: &mut Option<Pavement>,
                          pending_rings: &mut Vec<Ring>,
                          open_verts: &mut Vec<Vertex>,
                          pending_line_name: &mut Option<String>,
                          ap: &mut SourceAirport,
                          closed: bool| {
        if open_verts.is_empty() {
            return;
        }
        let ring = Ring { verts: std::mem::take(open_verts), closed };
        match ring_target {
            RingTarget::Pavement => pending_rings.push(ring),
            RingTarget::Line => {
                ap.painted_lines.push(PaintedLine { name: pending_line_name.clone(), ring, source: source::XPLANE });
            }
            RingTarget::Boundary => {
                if ap.boundary.is_none() {
                    ap.boundary = Some(ring);
                }
            }
            RingTarget::None => {}
        }
        let _ = pending_pavement;
    };

    // Finalise a pavement (110) when the next header arrives.
    fn finish_pavement(pending_pavement: &mut Option<Pavement>, pending_rings: &mut Vec<Ring>, ap: &mut SourceAirport) {
        if let Some(mut p) = pending_pavement.take() {
            let mut rings = std::mem::take(pending_rings);
            if rings.is_empty() {
                return;
            }
            p.outer = rings.remove(0);
            p.holes = rings;
            if p.outer.verts.len() >= 3 {
                ap.pavements.push(p);
            }
        } else {
            pending_rings.clear();
        }
    }

    while let Some(raw) = lines.next() {
        let line = raw.trim();
        if line.is_empty() || line == "I" || line == "A" {
            continue;
        }
        let mut it = line.split_whitespace();
        let code = match it.next().and_then(|c| c.parse::<u32>().ok()) {
            Some(c) => c,
            None => continue, // version line / comments
        };
        let rest: Vec<&str> = it.collect();

        // Header rows start a new airport.
        if matches!(code, 1 | 16 | 17) {
            if let Some(ap) = cur.as_mut() {
                flush_ring(&mut ring_target, &mut pending_pavement, &mut pending_rings, &mut open_verts, &mut pending_line_name, ap, false);
                finish_pavement(&mut pending_pavement, &mut pending_rings, ap);
                if collecting {
                    break;
                }
            }
            let icao = rest.get(3).copied().unwrap_or("");
            collecting = want_icao.map_or(true, |w| w.eq_ignore_ascii_case(icao));
            if !collecting {
                cur = None;
                continue;
            }
            let mut ap = SourceAirport::new(icao);
            ap.header.elevation_ft = rest.first().and_then(|s| s.parse().ok());
            ap.header.name = if rest.len() > 4 { Some(rest[4..].join(" ")) } else { None };
            ap.sources.push(source::XPLANE.to_string());
            cur = Some(ap);
            ring_target = RingTarget::None;
            continue;
        }
        if code == 99 {
            break;
        }
        let Some(ap) = cur.as_mut() else { continue };

        // Node rows.
        if (111..=116).contains(&code) {
            let v = parse_vertex(code, &rest)?;
            open_verts.push(v);
            match code {
                113 | 114 => flush_ring(&mut ring_target, &mut pending_pavement, &mut pending_rings, &mut open_verts, &mut pending_line_name, ap, true),
                115 | 116 => flush_ring(&mut ring_target, &mut pending_pavement, &mut pending_rings, &mut open_verts, &mut pending_line_name, ap, false),
                _ => {}
            }
            continue;
        }
        // Any non-node row ends open geometry.
        flush_ring(&mut ring_target, &mut pending_pavement, &mut pending_rings, &mut open_verts, &mut pending_line_name, ap, false);
        if code != 110 {
            finish_pavement(&mut pending_pavement, &mut pending_rings, ap);
        }

        match code {
            100 => {
                if rest.len() < 25 {
                    ap.warnings.push(format!("short runway row: {line}"));
                    continue;
                }
                let width = f(rest[0]);
                let surface = surftype::from_xplane(i(rest[1]));
                let shoulder_code = i(rest[2]);
                let cl = i(rest[4]) == 1;
                let edge = i(rest[5]);
                let end = |o: usize| -> RunwayEnd {
                    RunwayEnd {
                        ident: rest[o].to_string(),
                        pos: Coord { x: f(rest[o + 2]), y: f(rest[o + 1]) },
                        displaced_m: f(rest[o + 3]),
                        blastpad_m: f(rest[o + 4]),
                        marking: rwymktyp::from_xplane(i(rest[o + 5])),
                        approach_lights: i(rest[o + 6]),
                        tdz_lights: i(rest[o + 7]) == 1,
                        reil: i(rest[o + 8]),
                        tora_m: None,
                        toda_m: None,
                        asda_m: None,
                        lda_m: None,
                        tdze_ft: None,
                    }
                };
                ap.runways.push(Runway {
                    width_m: width,
                    surface,
                    shoulder_surface: if shoulder_code > 0 { Some(surftype::from_xplane(shoulder_code)) } else { None },
                    shoulder_width_m: None,
                    centerline_lights: cl,
                    edge_lights: edge,
                    ends: [end(7), end(16)],
                    stopway_m: [0.0, 0.0],
                    source: source::XPLANE,
                });
            }
            101 => {
                if rest.len() >= 8 {
                    ap.water_runways.push(WaterRunway {
                        width_m: f(rest[0]),
                        ends: [
                            (rest[2].to_string(), Coord { x: f(rest[4]), y: f(rest[3]) }),
                            (rest[5].to_string(), Coord { x: f(rest[7]), y: f(rest[6]) }),
                        ],
                    });
                }
            }
            102 => {
                if rest.len() >= 7 {
                    ap.helipads.push(Helipad {
                        ident: rest[0].to_string(),
                        pos: Coord { x: f(rest[2]), y: f(rest[1]) },
                        heading_deg: f(rest[3]),
                        length_m: f(rest[4]),
                        width_m: f(rest[5]),
                        surface: surftype::from_xplane(i(rest[6])),
                        source: source::XPLANE,
                    });
                }
            }
            110 => {
                finish_pavement(&mut pending_pavement, &mut pending_rings, ap);
                let name = if rest.len() > 3 { Some(rest[3..].join(" ")) } else { None };
                pending_pavement = Some(Pavement {
                    surface: surftype::from_xplane(rest.first().map(|s| i(s)).unwrap_or(0)),
                    hint: hint_from_name(name.as_deref()),
                    name,
                    outer: Ring { verts: vec![], closed: true },
                    holes: vec![],
                    source: source::XPLANE,
                });
                ring_target = RingTarget::Pavement;
            }
            120 => {
                pending_line_name = if rest.is_empty() { None } else { Some(rest.join(" ")) };
                ring_target = RingTarget::Line;
            }
            130 => {
                ring_target = RingTarget::Boundary;
            }
            18 => {
                if rest.len() >= 3 {
                    ap.lights.push(LightObject {
                        pos: Coord { x: f(rest[1]), y: f(rest[0]) },
                        kind: lighting::BEACON,
                        heading_deg: None,
                        glideslope_deg: None,
                        runway: None,
                        name: if rest.len() > 3 { Some(rest[3..].join(" ")) } else { None },
                        source: source::XPLANE,
                    });
                }
            }
            19 => {
                if rest.len() >= 3 {
                    ap.point_structures.push(PointStructure {
                        pos: Coord { x: f(rest[1]), y: f(rest[0]) },
                        kind: crate::model::codes::pntsttyp::WINDSOCK,
                        height_m: Some(6.0),
                        name: if rest.len() > 3 { Some(rest[3..].join(" ")) } else { None },
                        source: source::XPLANE,
                    });
                }
            }
            20 => {
                if rest.len() >= 6 {
                    ap.signs.push(Sign {
                        pos: Coord { x: f(rest[1]), y: f(rest[0]) },
                        heading_deg: f(rest[2]),
                        size: i(rest[4]),
                        raw: rest[5..].join(" "),
                        source: source::XPLANE,
                    });
                }
            }
            21 => {
                if rest.len() >= 5 {
                    let kind = match i(rest[2]) {
                        1 | 5 => lighting::VASI,
                        2 | 3 | 4 => lighting::PAPI,
                        6 => lighting::RUNWAY_GUARD,
                        7 | 8 => lighting::APAPI,
                        _ => lighting::UNKNOWN,
                    };
                    let rwy = rest.get(5).map(|s| s.to_string());
                    ap.lights.push(LightObject {
                        pos: Coord { x: f(rest[1]), y: f(rest[0]) },
                        kind,
                        heading_deg: Some(f(rest[3])),
                        glideslope_deg: Some(f(rest[4])).filter(|g| *g > 0.0),
                        runway: rwy,
                        name: if rest.len() > 6 { Some(rest[6..].join(" ")) } else { None },
                        source: source::XPLANE,
                    });
                }
            }
            50..=56 | 1050..=1056 => {
                if rest.len() >= 1 {
                    let raw = i(rest[0]) as f64;
                    let mhz = if code >= 1050 { raw / 1000.0 } else { raw / 100.0 };
                    let st = match code % 50 % 1000 {
                        0 => station::RECORDED,
                        1 => station::UNICOM,
                        2 => station::CLEARANCE,
                        3 => station::GROUND,
                        4 => station::TOWER,
                        5 => station::APPROACH,
                        _ => station::DEPARTURE,
                    };
                    ap.frequencies.push(Frequency { station: st, mhz, name: rest.get(1..).map(|s| s.join(" ")).unwrap_or_default() });
                }
            }
            1201 => {
                if rest.len() >= 4 {
                    ap.route_nodes.push(RouteNode {
                        id: i(rest[3]),
                        pos: Coord { x: f(rest[1]), y: f(rest[0]) },
                        usage: rest[2].to_string(),
                        name: rest.get(4..).map(|s| s.join(" ")).filter(|s| !s.is_empty()),
                    });
                }
            }
            1202 => {
                if rest.len() >= 4 {
                    ap.route_edges.push(RouteEdge {
                        from: i(rest[0]),
                        to: i(rest[1]),
                        oneway: rest[2] == "oneway",
                        restriction: rest[3].to_string(),
                        name: rest.get(4..).map(|s| s.join(" ")).filter(|s| !s.is_empty()),
                        active_zones: vec![],
                    });
                    last_edge = Some(ap.route_edges.len() - 1);
                }
            }
            1204 => {
                if let (Some(idx), true) = (last_edge, rest.len() >= 2) {
                    let rwys = rest[1].split(',').map(|s| s.to_string()).collect();
                    ap.route_edges[idx].active_zones.push((rest[0].to_string(), rwys));
                }
            }
            1300 => {
                if rest.len() >= 5 {
                    let kind = match rest[3] {
                        "gate" => StandKind::Gate,
                        "hangar" => StandKind::Hangar,
                        "tie_down" | "tie-down" => StandKind::TieDown,
                        _ => StandKind::Misc,
                    };
                    ap.stands.push(Stand {
                        name: rest.get(5..).map(|s| s.join(" ")).unwrap_or_default(),
                        pos: Coord { x: f(rest[1]), y: f(rest[0]) },
                        heading_deg: Some(f(rest[2])),
                        kind,
                        size_code: None,
                        operation: None,
                        airlines: vec![],
                        aircraft_types: rest[4].split('|').map(|s| s.to_string()).collect(),
                        jetway: None,
                        area: None,
                        source: source::XPLANE,
                    });
                    last_stand = Some(ap.stands.len() - 1);
                }
            }
            1301 => {
                if let Some(idx) = last_stand {
                    let s = &mut ap.stands[idx];
                    s.size_code = rest.first().and_then(|c| c.chars().next()).map(|c| c.to_ascii_uppercase());
                    s.operation = rest.get(1).map(|s| s.to_string());
                    s.airlines = rest.get(2..).map(|a| a.iter().map(|s| s.to_uppercase()).collect()).unwrap_or_default();
                }
            }
            1302 => {
                if rest.len() >= 2 {
                    let v = rest[1..].join(" ");
                    let h = &mut ap.header;
                    match rest[0] {
                        "icao_code" | "icao_id" => {
                            if h.icao.is_empty() {
                                h.icao = v;
                            }
                        }
                        "iata_code" => h.iata = Some(v),
                        "faa_code" => h.faa = Some(v),
                        "city" => h.city = Some(v),
                        "country" => h.country = Some(v),
                        "region_code" => h.region = Some(v),
                        "datum_lat" => {
                            let lat = f(&v);
                            h.arp = Some(Coord { x: h.arp.map(|c| c.x).unwrap_or(0.0), y: lat });
                        }
                        "datum_lon" => {
                            let lon = f(&v);
                            h.arp = Some(Coord { x: lon, y: h.arp.map(|c| c.y).unwrap_or(0.0) });
                        }
                        "transition_alt" => h.transition_alt_ft = v.parse().ok(),
                        "transition_level" => h.transition_level = Some(v),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    let mut ap = match cur {
        Some(ap) => ap,
        None => bail!("airport {} not found in apt.dat", want_icao.unwrap_or("<first>")),
    };
    flush_ring(&mut ring_target, &mut pending_pavement, &mut pending_rings, &mut open_verts, &mut pending_line_name, &mut ap, false);
    finish_pavement(&mut pending_pavement, &mut pending_rings, &mut ap);
    // datum_lat without datum_lon (or vice versa) is unusable.
    if let Some(arp) = ap.header.arp {
        if arp.x == 0.0 || arp.y == 0.0 {
            ap.header.arp = None;
        }
    }
    Ok(ap)
}

#[derive(Clone, Copy, PartialEq)]
enum RingTarget {
    None,
    Pavement,
    Line,
    Boundary,
}

fn f(s: &str) -> f64 {
    s.parse().unwrap_or(0.0)
}

fn i(s: &str) -> i64 {
    s.parse::<i64>().or_else(|_| s.parse::<f64>().map(|v| v as i64)).unwrap_or(0)
}

fn parse_vertex(code: u32, rest: &[&str]) -> Result<Vertex> {
    let has_ctrl = matches!(code, 112 | 114 | 116);
    let need = if has_ctrl { 4 } else { 2 };
    if rest.len() < need {
        bail!("short node row {code}: {rest:?}");
    }
    let pos = Coord { x: rest[1].parse().context("lon")?, y: rest[0].parse().context("lat")? };
    let ctrl = if has_ctrl { Some(Coord { x: f(rest[3]), y: f(rest[2]) }) } else { None };
    let (line, light) = if matches!(code, 115 | 116) {
        (0, 0)
    } else {
        let mut line = 0u16;
        let mut light = 0u16;
        for s in &rest[need..] {
            let v = i(s) as u16;
            if v >= 100 {
                light = v;
            } else {
                line = v;
            }
        }
        (line, light)
    };
    Ok(Vertex { pos, ctrl, line, light })
}

pub fn hint_from_name(name: Option<&str>) -> PavementHint {
    let Some(n) = name else { return PavementHint::Unknown };
    let n = n.to_ascii_lowercase();
    if n.contains("apron") || n.contains("ramp") || n.contains("gate") || n.contains("terminal") || n.contains("cargo") || n.contains("parking") || n.contains("stand") || n.contains("tie") || n.contains("hangar") {
        PavementHint::Apron
    } else if n.contains("road") || n.contains("street") {
        PavementHint::Road
    } else if n.contains("runway") || n.contains("rwy") {
        PavementHint::Runway
    } else if n.contains("heli") {
        PavementHint::Helipad
    } else if n.contains("taxi") || n.contains("twy") || n.len() <= 3 {
        PavementHint::Taxiway
    } else {
        PavementHint::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "I\n1200 Generated by WorldEditor\n\n1   6264 0 0 KTST Test Field\n1302 datum_lat 38.893888889\n1302 datum_lon -119.995333333\n1302 iata_code TST\n100 30.48 1 1 0.25 0 2 0  18  38.90531691 -119.99202409 244 59 3 9 0 0 36  38.88246472 -119.99872267 620 64 3 0 0 0\n110 1 0.25 192.8 A\n111  38.89275222 -119.99661584\n111  38.89278453 -119.99682291 3\n112  38.89274235 -119.99683338  38.89274000 -119.99684000 3 102\n113  38.89227926 -119.99658343\n120 Centerline A\n111  38.89309677 -119.99821119 1\n115  38.89159831 -119.99793883\n130 Airport Boundary\n111 38.88 -120.0\n111 38.91 -120.0\n111 38.91 -119.98\n113 38.88 -119.98\n20  38.88539941 -119.99749158 192.8 0 4 {@B}1{@@}7\n21  38.90058064 -119.99285462 2 192.8 3.00 18  PAPI-4L\n1050 124725 ASOS\n1054 118300 TWR\n1200 \n1201  38.89418877 -119.99762160 both 0 _start\n1201  38.89646006 -119.99698065 both 1 _split\n1202 0 1 twoway taxiway_B \n1204 departure 18,36\n1300  38.89218390 -119.99816836 23.0 gate jets|turboprops Gate 1\n1301 C airline dal aal\n99\n";

    #[test]
    fn parses_sample_airport() {
        let ap = parse(SAMPLE, None).unwrap();
        assert_eq!(ap.header.icao, "KTST");
        assert_eq!(ap.header.name.as_deref(), Some("Test Field"));
        assert_eq!(ap.header.iata.as_deref(), Some("TST"));
        let arp = ap.header.arp.unwrap();
        assert!((arp.y - 38.893888889).abs() < 1e-9 && (arp.x + 119.995333333).abs() < 1e-9);
        assert_eq!(ap.runways.len(), 1);
        let r = &ap.runways[0];
        assert_eq!(r.ends[0].ident, "18");
        assert_eq!(r.ends[1].ident, "36");
        assert!((r.ends[1].displaced_m - 620.0).abs() < 1e-9);
        assert_eq!(r.ends[0].marking, rwymktyp::PRECISION);
        assert_eq!(ap.pavements.len(), 1);
        assert_eq!(ap.pavements[0].outer.verts.len(), 4);
        assert_eq!(ap.pavements[0].outer.verts[2].line, 3);
        assert_eq!(ap.pavements[0].outer.verts[2].light, 102);
        assert!(ap.pavements[0].outer.verts[2].ctrl.is_some());
        assert_eq!(ap.pavements[0].hint, PavementHint::Taxiway);
        assert_eq!(ap.painted_lines.len(), 1);
        assert!(!ap.painted_lines[0].ring.closed);
        assert_eq!(ap.painted_lines[0].ring.verts[0].line, 1);
        assert!(ap.boundary.is_some());
        assert_eq!(ap.signs.len(), 1);
        assert_eq!(ap.signs[0].raw, "{@B}1{@@}7");
        assert_eq!(ap.lights.len(), 1);
        assert_eq!(ap.lights[0].kind, lighting::PAPI);
        assert_eq!(ap.lights[0].runway.as_deref(), Some("18"));
        assert_eq!(ap.frequencies.len(), 2);
        assert!((ap.frequencies[1].mhz - 118.3).abs() < 1e-9);
        assert_eq!(ap.route_nodes.len(), 2);
        assert_eq!(ap.route_edges.len(), 1);
        assert_eq!(ap.route_edges[0].active_zones[0].1, vec!["18", "36"]);
        assert_eq!(ap.stands.len(), 1);
        assert_eq!(ap.stands[0].name, "Gate 1");
        assert_eq!(ap.stands[0].size_code, Some('C'));
        assert_eq!(ap.stands[0].airlines, vec!["DAL", "AAL"]);
    }

    #[test]
    fn selects_requested_airport_from_multi_airport_file() {
        let two = format!("{}\n1 10 0 0 KAAA Other\n100 30 1 0 0.25 0 0 0 09 1 2 0 0 0 0 0 0 27 1 2.1 0 0 0 0 0 0\n99\n", SAMPLE.trim_end_matches("99\n"));
        let ap = parse(&two, Some("KAAA")).unwrap();
        assert_eq!(ap.header.icao, "KAAA");
        assert_eq!(ap.runways.len(), 1);
        assert!(parse(&two, Some("ZZZZ")).is_err());
    }
}
