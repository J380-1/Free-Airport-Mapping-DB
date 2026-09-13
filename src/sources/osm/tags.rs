//! Map OSM elements (via tags) into the intermediate model.

use super::elements::{PolyGeom, Store, Tags, Way};
use crate::ir::*;
use crate::model::codes::{lighting, linsttyp, plysttyp, pntsttyp, source, surftype};
use geo_types::Coord;

fn t<'a>(tags: &'a Tags, k: &str) -> Option<&'a str> {
    tags.get(k).map(String::as_str).filter(|v| !v.is_empty())
}

fn parse_len_m(v: &str) -> Option<f64> {
    let v = v.trim().to_ascii_lowercase();
    if let Some(n) = v.strip_suffix(" m").or_else(|| v.strip_suffix('m')) {
        return n.trim().parse().ok();
    }
    if let Some(n) = v.strip_suffix(" ft").or_else(|| v.strip_suffix("ft")).or_else(|| v.strip_suffix('\'')) {
        return n.trim().parse::<f64>().ok().map(|f| f * 0.3048);
    }
    v.parse().ok()
}

fn name_of(tags: &Tags) -> Option<String> {
    t(tags, "ref").or_else(|| t(tags, "name")).map(str::to_string)
}

fn surface_of(tags: &Tags) -> Option<i64> {
    t(tags, "surface").map(surftype::from_osm)
}

fn height_of(tags: &Tags) -> (Option<f64>, Option<f64>) {
    let h = t(tags, "height").and_then(parse_len_m);
    let levels = t(tags, "building:levels").and_then(|v| v.parse::<f64>().ok());
    (h.or(levels.map(|l| l * 3.5)), levels)
}

fn building_kind(tags: &Tags) -> i64 {
    match t(tags, "aeroway") {
        Some("terminal") => return plysttyp::TERMINAL,
        Some("hangar") => return plysttyp::HANGAR,
        Some("tower") | Some("control_tower") => return plysttyp::CONTROL_TOWER,
        Some("fuel") => return plysttyp::FUEL_FARM,
        _ => {}
    }
    match t(tags, "building") {
        Some("terminal") => plysttyp::TERMINAL,
        Some("hangar") => plysttyp::HANGAR,
        Some("control_tower") => plysttyp::CONTROL_TOWER,
        Some("parking") | Some("garage") | Some("garages") => plysttyp::PARKING_GARAGE,
        Some("industrial") | Some("warehouse") => plysttyp::INDUSTRIAL,
        _ => {
            if t(tags, "man_made") == Some("storage_tank") || t(tags, "man_made") == Some("silo") {
                plysttyp::FUEL_FARM
            } else if t(tags, "man_made") == Some("tower") && t(tags, "tower:type").map_or(false, |v| v.contains("control")) {
                plysttyp::CONTROL_TOWER
            } else {
                plysttyp::BUILDING
            }
        }
    }
}

fn point_kind(tags: &Tags) -> Option<i64> {
    if let Some(a) = t(tags, "aeroway") {
        return match a {
            "windsock" => Some(pntsttyp::WINDSOCK),
            "navigationaid" | "papi" | "vasi" => None, // handled as lighting
            "tower" | "control_tower" => Some(pntsttyp::TOWER),
            _ => None,
        };
    }
    match t(tags, "man_made") {
        Some("tower") | Some("communications_tower") | Some("water_tower") | Some("lighthouse") => Some(pntsttyp::TOWER),
        Some("mast") | Some("flagpole") => Some(pntsttyp::MAST),
        Some("chimney") => Some(pntsttyp::CHIMNEY),
        Some("antenna") => Some(pntsttyp::ANTENNA),
        Some("storage_tank") | Some("silo") => Some(pntsttyp::TANK),
        Some("utility_pole") => Some(pntsttyp::POLE),
        _ => match t(tags, "power") {
            Some("tower") | Some("pole") | Some("portal") => Some(pntsttyp::POLE),
            _ => None, // trees and street lamps are deliberately not obstacles here
        },
    }
}

fn line_kind(tags: &Tags) -> Option<i64> {
    match t(tags, "barrier") {
        Some("fence") | Some("chain_link") => Some(linsttyp::FENCE),
        Some("wall") | Some("retaining_wall") | Some("city_wall") => Some(linsttyp::WALL),
        Some("hedge") => Some(linsttyp::HEDGE),
        _ => match t(tags, "power") {
            Some("line") | Some("minor_line") => Some(linsttyp::POWER_LINE),
            Some("cable") => Some(linsttyp::CABLE),
            _ => match t(tags, "aerialway") {
                Some(_) => Some(linsttyp::CABLE),
                _ => None,
            },
        },
    }
}

fn is_deicing(tags: &Tags) -> bool {
    t(tags, "apron") == Some("deicing")
        || t(tags, "deicing") == Some("yes")
        || t(tags, "aeroway") == Some("deicing")
        || t(tags, "name").map_or(false, |n| n.to_ascii_lowercase().contains("deic") || n.to_ascii_lowercase().contains("de-ic"))
}

fn road_width(tags: &Tags) -> f64 {
    if let Some(w) = t(tags, "width").and_then(parse_len_m) {
        return w.clamp(2.0, 30.0);
    }
    let lanes = t(tags, "lanes").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
    if lanes > 0.0 {
        return (lanes * 3.25).clamp(3.0, 30.0);
    }
    match t(tags, "highway") {
        Some("service") | Some("track") => 5.0,
        Some("residential") | Some("unclassified") => 6.5,
        Some("tertiary") | Some("secondary") => 8.0,
        Some("primary") | Some("trunk") | Some("motorway") => 12.0,
        _ => 5.0,
    }
}

fn taxiway_width(tags: &Tags) -> Option<f64> {
    t(tags, "width").and_then(parse_len_m)
}

/// Convert every element in the store into IR items. Spatial filtering (inside the
/// aerodrome) is done later by the builder, which knows the pavement extent.
pub fn store_to_ir(store: &Store, icao: &str) -> SourceAirport {
    let mut ap = SourceAirport::new(icao);
    ap.sources.push(source::OSM.to_string());

    // Nodes with tags.
    for (id, tags) in &store.node_tags {
        let Some(pos) = store.nodes.get(id).copied() else { continue };
        match t(tags, "aeroway") {
            Some("parking_position") => ap.stands.push(stand_from(tags, pos, None)),
            Some("holding_position") => ap.semantic_lines.push(SemanticLine {
                kind: match t(tags, "holding_position:type") {
                    Some("ils") | Some("cat_ii") | Some("cat_iii") | Some("cat_ii_iii") => LineKind::IlsHold,
                    Some("intermediate") | Some("taxiway") => LineKind::IntersectionHold,
                    _ => LineKind::RunwayHold,
                },
                name: name_of(tags),
                width_m: None,
                pts: vec![pos],
                bridge: false,
                source: source::OSM,
            }),
            Some("beacon") | Some("aerodrome_beacon") => ap.lights.push(LightObject { pos, kind: lighting::BEACON, heading_deg: None, glideslope_deg: None, runway: None, name: name_of(tags), source: source::OSM }),
            Some("papi") => ap.lights.push(LightObject { pos, kind: lighting::PAPI, heading_deg: None, glideslope_deg: None, runway: name_of(tags), name: None, source: source::OSM }),
            Some("vasi") => ap.lights.push(LightObject { pos, kind: lighting::VASI, heading_deg: None, glideslope_deg: None, runway: name_of(tags), name: None, source: source::OSM }),
            Some("navigationaid") => {
                // Only visual approach aids; individually mapped taxiway/edge lights are skipped.
                let na = t(tags, "navigationaid").unwrap_or("").to_ascii_lowercase();
                let kind = if na.contains("papi") { lighting::PAPI } else if na.contains("vasi") { lighting::VASI } else if na.contains("als") || na.contains("approach") { lighting::APPROACH } else if na.contains("reil") { lighting::REIL } else { continue };
                ap.lights.push(LightObject { pos, kind, heading_deg: t(tags, "direction").and_then(|v| v.parse().ok()), glideslope_deg: None, runway: t(tags, "ref").map(str::to_string), name: Some(na.clone()), source: source::OSM });
            }
            Some("helipad") => ap.helipads.push(Helipad { ident: name_of(tags).unwrap_or_else(|| "H".into()), pos, heading_deg: 0.0, length_m: 20.0, width_m: 20.0, surface: surface_of(tags).unwrap_or(surftype::UNKNOWN), source: source::OSM }),
            Some("aerodrome") => {
                if ap.header.name.is_none() {
                    ap.header.name = t(tags, "name").map(str::to_string);
                }
                if ap.header.iata.is_none() {
                    ap.header.iata = t(tags, "iata").map(str::to_string);
                }
            }
            _ => {
                if let Some(k) = point_kind(tags) {
                    ap.point_structures.push(PointStructure { pos, kind: k, height_m: height_of(tags).0, name: t(tags, "name").map(str::to_string), source: source::OSM });
                }
            }
        }
    }

    // Ways.
    for w in store.ways.values() {
        let coords = store.way_coords(w);
        if coords.len() < 2 {
            continue;
        }
        if store.way_is_area(w) {
            area_to_ir(&mut ap, &w.tags, PolyGeom { outer: coords, holes: vec![] }, w.id);
        } else {
            line_to_ir(&mut ap, w, coords);
        }
    }

    // Relations (multipolygons and aerodrome boundaries).
    for r in store.relations.values() {
        let ty = t(&r.tags, "type");
        if ty != Some("multipolygon") && ty != Some("boundary") {
            continue;
        }
        for pg in store.relation_polygons(r) {
            area_to_ir(&mut ap, &r.tags, pg, r.id);
        }
    }
    ap
}

fn stand_from(tags: &Tags, pos: Coord<f64>, area: Option<Vec<Coord<f64>>>) -> Stand {
    let heading = t(tags, "direction").or_else(|| t(tags, "heading")).and_then(|v| v.parse::<f64>().ok());
    let size = t(tags, "aircraft:size").or_else(|| t(tags, "icao:size")).or_else(|| t(tags, "parking_position:size")).and_then(|v| v.chars().next()).map(|c| c.to_ascii_uppercase()).filter(|c| ('A'..='F').contains(c));
    Stand {
        name: name_of(tags).unwrap_or_default(),
        pos,
        heading_deg: heading,
        kind: if t(tags, "parking_position") == Some("hangar") { StandKind::Hangar } else { StandKind::Gate },
        size_code: size,
        operation: t(tags, "operator").map(str::to_string),
        airlines: t(tags, "airline").map(|a| a.split(';').map(|s| s.trim().to_uppercase()).collect()).unwrap_or_default(),
        aircraft_types: vec![],
        jetway: t(tags, "jet_bridge").or_else(|| t(tags, "jetway")).map(|v| v == "yes"),
        area,
        source: source::OSM,
    }
}

fn line_to_ir(ap: &mut SourceAirport, w: &Way, coords: Vec<Coord<f64>>) {
    let tags = &w.tags;
    let bridge = t(tags, "bridge").map_or(false, |v| v != "no");
    match t(tags, "aeroway") {
        Some("taxiway") | Some("taxilane") => {
            ap.semantic_lines.push(SemanticLine { kind: LineKind::TaxiCenterline, name: name_of(tags), width_m: taxiway_width(tags), pts: coords, bridge, source: source::OSM });
        }
        Some("runway") => {
            // Fallback runway from an OSM centreline.
            let width = t(tags, "width").and_then(parse_len_m).unwrap_or(45.0);
            let refs = t(tags, "ref").unwrap_or("");
            let mut idents = refs.split('/').map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
            let a = idents.next().unwrap_or_default();
            let b = idents.next().unwrap_or_default();
            let end = |ident: String, pos: Coord<f64>| RunwayEnd { ident, pos, displaced_m: 0.0, blastpad_m: 0.0, marking: 0, approach_lights: 0, tdz_lights: false, reil: 0, tora_m: None, toda_m: None, asda_m: None, lda_m: None, tdze_ft: None };
            ap.runways.push(Runway {
                width_m: width,
                surface: surface_of(tags).unwrap_or(surftype::UNKNOWN),
                shoulder_surface: None,
                shoulder_width_m: None,
                centerline_lights: false,
                edge_lights: 0,
                ends: [end(a, coords[0]), end(b, *coords.last().unwrap())],
                stopway_m: [0.0, 0.0],
                source: source::OSM,
            });
        }
        Some("stopway") => {}
        _ => {
            if let Some(hw) = t(tags, "highway") {
                let aisle = t(tags, "service").map_or(false, |v| matches!(v, "parking_aisle" | "driveway" | "drive-through" | "emergency_access"));
                if matches!(hw, "service" | "unclassified" | "living_street" | "track") && !aisle {
                    ap.semantic_lines.push(SemanticLine { kind: LineKind::RoadCenter, name: t(tags, "name").map(str::to_string), width_m: Some(road_width(tags)), pts: coords, bridge, source: source::OSM });
                }
            } else if let Some(k) = line_kind(tags) {
                ap.line_structures.push(LineStructure { pts: coords, kind: k, height_m: height_of(tags).0, name: t(tags, "name").map(str::to_string), source: source::OSM });
            }
        }
    }
}

fn area_to_ir(ap: &mut SourceAirport, tags: &Tags, pg: PolyGeom, id: i64) {
    let name = name_of(tags);
    let mk = |kind: AreaKind, name: Option<String>| Area { kind, outer: pg.outer.clone(), holes: pg.holes.clone(), name, surface: surface_of(tags), reference: Some(format!("osm:{id}")), source: source::OSM };
    match t(tags, "aeroway") {
        Some("aerodrome") => {
            if ap.header.name.is_none() {
                ap.header.name = t(tags, "name").map(str::to_string);
            }
            if ap.header.iata.is_none() {
                ap.header.iata = t(tags, "iata").map(str::to_string);
            }
            ap.areas.push(mk(AreaKind::Aerodrome, name));
            return;
        }
        Some("apron") => {
            ap.areas.push(mk(if is_deicing(tags) { AreaKind::Deicing } else { AreaKind::Apron }, name));
            return;
        }
        Some("taxiway") => {
            ap.areas.push(mk(AreaKind::Taxiway, name));
            return;
        }
        Some("runway") => {
            ap.areas.push(mk(AreaKind::Runway, name));
            return;
        }
        Some("stopway") => {
            ap.areas.push(mk(AreaKind::Stopway, name));
            return;
        }
        Some("blast_pad") | Some("blastpad") => {
            ap.areas.push(mk(AreaKind::Blastpad, name));
            return;
        }
        Some("helipad") => {
            let c = centroid(&pg.outer);
            ap.helipads.push(Helipad { ident: name.clone().unwrap_or_else(|| "H".into()), pos: c, heading_deg: 0.0, length_m: 0.0, width_m: 0.0, surface: surface_of(tags).unwrap_or(surftype::UNKNOWN), source: source::OSM });
            ap.areas.push(mk(AreaKind::Helipad, name));
            return;
        }
        Some("parking_position") => {
            let c = centroid(&pg.outer);
            ap.stands.push(stand_from(tags, c, Some(pg.outer.clone())));
            return;
        }
        Some("terminal") | Some("hangar") | Some("tower") | Some("control_tower") | Some("fuel") => {
            let (h, l) = height_of(tags);
            ap.buildings.push(Building { outer: pg.outer, holes: pg.holes, name: t(tags, "name").map(str::to_string), kind: building_kind(tags), height_m: h, levels: l, source: source::OSM });
            return;
        }
        _ => {}
    }
    if is_deicing(tags) {
        ap.areas.push(mk(AreaKind::Deicing, name));
        return;
    }
    if tags.contains_key("building") && t(tags, "building") != Some("no") {
        let (h, l) = height_of(tags);
        ap.buildings.push(Building { outer: pg.outer, holes: pg.holes, name: t(tags, "name").map(str::to_string), kind: building_kind(tags), height_m: h, levels: l, source: source::OSM });
        return;
    }
    if t(tags, "man_made") == Some("storage_tank") {
        let (h, l) = height_of(tags);
        ap.buildings.push(Building { outer: pg.outer, holes: pg.holes, name: t(tags, "name").map(str::to_string), kind: plysttyp::FUEL_FARM, height_m: h, levels: l, source: source::OSM });
        return;
    }
    if t(tags, "natural") == Some("water") || tags.contains_key("water") || t(tags, "landuse") == Some("reservoir") || t(tags, "landuse") == Some("basin") {
        ap.areas.push(mk(AreaKind::Water, name));
        return;
    }
    if t(tags, "landuse") == Some("construction") || tags.contains_key("construction") {
        ap.areas.push(mk(AreaKind::Construction, name));
        return;
    }
    if t(tags, "amenity") == Some("parking") && t(tags, "parking") != Some("multi-storey") {
        ap.areas.push(mk(AreaKind::ServiceArea, name));
    }
}

fn centroid(ring: &[Coord<f64>]) -> Coord<f64> {
    let n = ring.len().saturating_sub(1).max(1) as f64;
    let (sx, sy) = ring.iter().take(ring.len().saturating_sub(1).max(1)).fold((0.0, 0.0), |(a, b), c| (a + c.x, b + c.y));
    Coord { x: sx / n, y: sy / n }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::osm::elements::{Member, Relation};

    fn tags(kv: &[(&str, &str)]) -> Tags {
        kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn maps_common_airport_tags() {
        let mut s = Store::default();
        let pts = [(0.0, 0.0), (0.001, 0.0), (0.001, 0.001), (0.0, 0.001), (0.0005, 0.0005), (0.002, 0.0)];
        for (i, (x, y)) in pts.iter().enumerate() {
            s.nodes.insert(i as i64, Coord { x: *x, y: *y });
        }
        s.node_tags.insert(4, tags(&[("aeroway", "parking_position"), ("ref", "A12"), ("aircraft:size", "c")]));
        s.node_tags.insert(5, tags(&[("man_made", "mast"), ("height", "30 m")]));
        s.ways.insert(1, Way { id: 1, nodes: vec![0, 1, 2, 3, 0], tags: tags(&[("aeroway", "apron"), ("surface", "concrete")]) });
        s.ways.insert(2, Way { id: 2, nodes: vec![0, 1, 2, 3, 0], tags: tags(&[("building", "yes"), ("aeroway", "terminal"), ("building:levels", "3")]) });
        s.ways.insert(3, Way { id: 3, nodes: vec![0, 5], tags: tags(&[("aeroway", "taxiway"), ("ref", "B"), ("width", "23"), ("bridge", "yes")]) });
        s.ways.insert(4, Way { id: 4, nodes: vec![0, 5], tags: tags(&[("highway", "service"), ("lanes", "2")]) });
        s.ways.insert(5, Way { id: 5, nodes: vec![0, 1, 2], tags: tags(&[("barrier", "fence")]) });
        s.ways.insert(6, Way { id: 6, nodes: vec![0, 5], tags: tags(&[("aeroway", "runway"), ("ref", "09/27"), ("width", "45")]) });
        s.ways.insert(7, Way { id: 7, nodes: vec![0, 1, 2, 3, 0], tags: tags(&[("aeroway", "apron"), ("apron", "deicing")]) });
        s.relations.insert(9, Relation { id: 9, members: vec![Member { kind: 'w', id: 1, role: "outer".into() }], tags: tags(&[("type", "multipolygon"), ("aeroway", "aerodrome"), ("name", "Test Intl"), ("iata", "TST")]) });
        let ap = store_to_ir(&s, "ZZZZ");
        assert_eq!(ap.stands.len(), 1);
        assert_eq!(ap.stands[0].name, "A12");
        assert_eq!(ap.stands[0].size_code, Some('C'));
        assert_eq!(ap.point_structures.len(), 1);
        assert_eq!(ap.point_structures[0].height_m, Some(30.0));
        assert_eq!(ap.areas.iter().filter(|a| a.kind == AreaKind::Apron).count(), 1);
        assert_eq!(ap.areas.iter().filter(|a| a.kind == AreaKind::Deicing).count(), 1);
        assert_eq!(ap.areas.iter().filter(|a| a.kind == AreaKind::Aerodrome).count(), 1);
        assert_eq!(ap.buildings.len(), 1);
        assert_eq!(ap.buildings[0].kind, plysttyp::TERMINAL);
        assert_eq!(ap.buildings[0].height_m, Some(10.5));
        let twy: Vec<_> = ap.semantic_lines.iter().filter(|l| l.kind == LineKind::TaxiCenterline).collect();
        assert_eq!(twy.len(), 1);
        assert_eq!(twy[0].width_m, Some(23.0));
        assert!(twy[0].bridge);
        let road: Vec<_> = ap.semantic_lines.iter().filter(|l| l.kind == LineKind::RoadCenter).collect();
        assert_eq!(road[0].width_m, Some(6.5));
        assert_eq!(ap.line_structures.len(), 1);
        assert_eq!(ap.runways.len(), 1);
        assert_eq!(ap.runways[0].ends[1].ident, "27");
        assert_eq!(ap.header.name.as_deref(), Some("Test Intl"));
        assert_eq!(ap.header.iata.as_deref(), Some("TST"));
    }
}
