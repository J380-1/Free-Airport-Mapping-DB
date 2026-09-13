//! Intermediate representation produced by every source. Coordinates are WGS84
//! (`x = lon`, `y = lat`) here; the builder projects them into the local frame.

use geo_types::Coord;
use serde::Serialize;

pub type LonLat = Coord<f64>;

#[derive(Debug, Clone, Default, Serialize)]
pub struct AirportHeader {
    pub icao: String,
    pub iata: Option<String>,
    pub faa: Option<String>,
    pub name: Option<String>,
    pub city: Option<String>,
    pub country: Option<String>,
    pub region: Option<String>,
    pub arp: Option<LonLat>,
    pub elevation_ft: Option<f64>,
    pub transition_alt_ft: Option<f64>,
    pub transition_level: Option<String>,
    pub mag_var: Option<f64>,
}

/// One end of a runway (apt.dat row 100 semantics).
#[derive(Debug, Clone, Serialize)]
pub struct RunwayEnd {
    pub ident: String,
    pub pos: LonLat,
    pub displaced_m: f64,
    pub blastpad_m: f64,
    /// DO-272 `rwymktyp`.
    pub marking: i64,
    pub approach_lights: i64,
    pub tdz_lights: bool,
    pub reil: i64,
    /// Declared distances (metres) when a source publishes them.
    pub tora_m: Option<f64>,
    pub toda_m: Option<f64>,
    pub asda_m: Option<f64>,
    pub lda_m: Option<f64>,
    pub tdze_ft: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Runway {
    pub width_m: f64,
    /// DO-272 `surftype`.
    pub surface: i64,
    pub shoulder_surface: Option<i64>,
    pub shoulder_width_m: Option<f64>,
    pub centerline_lights: bool,
    pub edge_lights: i64,
    pub ends: [RunwayEnd; 2],
    /// Stopway length per end in metres, if known (FAA/OSM).
    pub stopway_m: [f64; 2],
    pub source: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct WaterRunway {
    pub width_m: f64,
    pub ends: [(String, LonLat); 2],
}

#[derive(Debug, Clone, Serialize)]
pub struct Helipad {
    pub ident: String,
    pub pos: LonLat,
    pub heading_deg: f64,
    pub length_m: f64,
    pub width_m: f64,
    pub surface: i64,
    pub source: &'static str,
}

/// A vertex of a pavement/line ring. `line`/`light` are apt.dat style codes that apply
/// to the segment *starting* at this vertex.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Vertex {
    pub pos: LonLat,
    pub ctrl: Option<LonLat>,
    pub line: u16,
    pub light: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct Ring {
    pub verts: Vec<Vertex>,
    pub closed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum PavementHint {
    Unknown,
    Taxiway,
    Apron,
    Runway,
    Helipad,
    Road,
}

#[derive(Debug, Clone, Serialize)]
pub struct Pavement {
    pub surface: i64,
    pub name: Option<String>,
    pub hint: PavementHint,
    pub outer: Ring,
    pub holes: Vec<Ring>,
    pub source: &'static str,
}

/// Painted line string (apt.dat row 120): line/light codes live on the vertices.
#[derive(Debug, Clone, Serialize)]
pub struct PaintedLine {
    pub name: Option<String>,
    pub ring: Ring,
    pub source: &'static str,
}

/// A centreline supplied as a plain line with a semantic (OSM taxiway ways, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum LineKind {
    TaxiCenterline,
    RunwayHold,
    IlsHold,
    IntersectionHold,
    TaxiEdge,
    RoadEdge,
    RoadCenter,
    Other,
}

#[derive(Debug, Clone, Serialize)]
pub struct SemanticLine {
    pub kind: LineKind,
    pub name: Option<String>,
    pub width_m: Option<f64>,
    pub pts: Vec<LonLat>,
    pub bridge: bool,
    pub source: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum StandKind {
    Gate,
    Hangar,
    Misc,
    TieDown,
}

#[derive(Debug, Clone, Serialize)]
pub struct Stand {
    pub name: String,
    pub pos: LonLat,
    pub heading_deg: Option<f64>,
    pub kind: StandKind,
    /// ICAO aerodrome reference code letter A-F.
    pub size_code: Option<char>,
    pub operation: Option<String>,
    pub airlines: Vec<String>,
    pub aircraft_types: Vec<String>,
    pub jetway: Option<bool>,
    pub area: Option<Vec<LonLat>>,
    pub source: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct RouteNode {
    pub id: i64,
    pub pos: LonLat,
    pub usage: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RouteEdge {
    pub from: i64,
    pub to: i64,
    pub oneway: bool,
    /// "runway" or "taxiway_X"
    pub restriction: String,
    pub name: Option<String>,
    /// Active zones: (zone type, runway idents).
    pub active_zones: Vec<(String, Vec<String>)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Sign {
    pub pos: LonLat,
    pub heading_deg: f64,
    pub size: i64,
    pub raw: String,
    pub source: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct LightObject {
    pub pos: LonLat,
    /// DO-272 `lstype` code.
    pub kind: i64,
    pub heading_deg: Option<f64>,
    pub glideslope_deg: Option<f64>,
    pub runway: Option<String>,
    pub name: Option<String>,
    pub source: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct Frequency {
    /// DO-272 `station` code.
    pub station: i64,
    pub mhz: f64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Building {
    pub outer: Vec<LonLat>,
    pub holes: Vec<Vec<LonLat>>,
    pub name: Option<String>,
    /// DO-272 `plysttyp`.
    pub kind: i64,
    pub height_m: Option<f64>,
    pub levels: Option<f64>,
    pub source: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct PointStructure {
    pub pos: LonLat,
    pub kind: i64,
    pub height_m: Option<f64>,
    pub name: Option<String>,
    pub source: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct LineStructure {
    pub pts: Vec<LonLat>,
    pub kind: i64,
    pub height_m: Option<f64>,
    pub name: Option<String>,
    pub source: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AreaKind {
    Apron,
    Water,
    Construction,
    Deicing,
    Stopway,
    Blastpad,
    Hotspot,
    Aerodrome,
    Helipad,
    Taxiway,
    Runway,
    ServiceArea,
}

#[derive(Debug, Clone, Serialize)]
pub struct Area {
    pub kind: AreaKind,
    pub outer: Vec<LonLat>,
    pub holes: Vec<Vec<LonLat>>,
    pub name: Option<String>,
    pub surface: Option<i64>,
    pub reference: Option<String>,
    pub source: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArrestingSystem {
    pub runway_end: String,
    pub kind: String,
    /// Distance from the runway end in metres, if known.
    pub distance_m: Option<f64>,
    pub source: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct Lahso {
    pub runway_end: String,
    pub available_m: f64,
    pub intersecting: Option<String>,
    pub source: &'static str,
}

/// Everything one or more sources know about an airport, before conflation.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SourceAirport {
    pub header: AirportHeader,
    pub runways: Vec<Runway>,
    pub water_runways: Vec<WaterRunway>,
    pub helipads: Vec<Helipad>,
    pub pavements: Vec<Pavement>,
    pub painted_lines: Vec<PaintedLine>,
    pub semantic_lines: Vec<SemanticLine>,
    pub boundary: Option<Ring>,
    pub stands: Vec<Stand>,
    pub route_nodes: Vec<RouteNode>,
    pub route_edges: Vec<RouteEdge>,
    pub signs: Vec<Sign>,
    pub lights: Vec<LightObject>,
    pub frequencies: Vec<Frequency>,
    pub buildings: Vec<Building>,
    pub point_structures: Vec<PointStructure>,
    pub line_structures: Vec<LineStructure>,
    pub areas: Vec<Area>,
    pub arresting: Vec<ArrestingSystem>,
    pub lahso: Vec<Lahso>,
    /// Which sources contributed (for the manifest).
    pub sources: Vec<String>,
    pub warnings: Vec<String>,
}

impl SourceAirport {
    pub fn new(icao: &str) -> Self {
        Self { header: AirportHeader { icao: icao.to_uppercase(), ..Default::default() }, ..Default::default() }
    }

    /// Merge `other` into `self`. Header fields fill gaps only; collections append.
    pub fn absorb(&mut self, other: SourceAirport) {
        let h = &mut self.header;
        let o = other.header;
        macro_rules! fill { ($($f:ident),*) => { $( if h.$f.is_none() { h.$f = o.$f; } )* } }
        fill!(iata, faa, name, city, country, region, arp, elevation_ft, transition_alt_ft, transition_level, mag_var);
        self.runways.extend(other.runways);
        self.water_runways.extend(other.water_runways);
        self.helipads.extend(other.helipads);
        self.pavements.extend(other.pavements);
        self.painted_lines.extend(other.painted_lines);
        self.semantic_lines.extend(other.semantic_lines);
        if self.boundary.is_none() {
            self.boundary = other.boundary;
        }
        self.stands.extend(other.stands);
        self.route_nodes.extend(other.route_nodes);
        self.route_edges.extend(other.route_edges);
        self.signs.extend(other.signs);
        self.lights.extend(other.lights);
        self.frequencies.extend(other.frequencies);
        self.buildings.extend(other.buildings);
        self.point_structures.extend(other.point_structures);
        self.line_structures.extend(other.line_structures);
        self.areas.extend(other.areas);
        self.arresting.extend(other.arresting);
        self.lahso.extend(other.lahso);
        self.sources.extend(other.sources);
        self.warnings.extend(other.warnings);
    }

    /// Bounding box (min_lon, min_lat, max_lon, max_lat) over all geometry.
    pub fn bbox(&self) -> Option<(f64, f64, f64, f64)> {
        let mut pts: Vec<LonLat> = Vec::new();
        for r in &self.runways {
            pts.push(r.ends[0].pos);
            pts.push(r.ends[1].pos);
        }
        for p in &self.pavements {
            pts.extend(p.outer.verts.iter().map(|v| v.pos));
        }
        pts.extend(self.stands.iter().map(|s| s.pos));
        pts.extend(self.route_nodes.iter().map(|n| n.pos));
        pts.extend(self.helipads.iter().map(|h| h.pos));
        for l in &self.semantic_lines {
            pts.extend(l.pts.iter().copied());
        }
        for a in &self.areas {
            if a.kind == AreaKind::Aerodrome {
                pts.extend(a.outer.iter().copied());
            }
        }
        if let Some(bd) = &self.boundary {
            pts.extend(bd.verts.iter().map(|v| v.pos));
        }
        if pts.is_empty() {
            pts.extend(self.header.arp);
        }
        let mut it = pts.into_iter();
        let first = it.next()?;
        Some(it.fold((first.x, first.y, first.x, first.y), |(a, b, c, d), p| (a.min(p.x), b.min(p.y), c.max(p.x), d.max(p.y))))
    }
}
