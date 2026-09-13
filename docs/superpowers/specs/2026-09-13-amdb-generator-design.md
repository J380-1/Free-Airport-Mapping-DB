# AMDB Generator (amdbgen) — Design

Date: 2026-09-13

## Goal

A Rust CLI that builds a Navigraph-style Airport Mapping Database (AMDB) for any
airport from free, key-less sources, producing exactly the files an OANS-style
nav-map plugin needs: one folder per ICAO containing every DO-272 / AMXM feature
layer as GeoJSON and as PBF (Geobuf), in WGS84 or in a local metres-from-ARP
azimuthal-equidistant frame.

## Sources (all free, no API key)

| Source | Transport | Licence | Provides |
|---|---|---|---|
| X-Plane Scenery Gateway | `GET /apiv1/airport/{icao}`, `GET /apiv1/scenery/{id}` (base64 zip with apt.dat); optional local X-Plane install | Gateway ToS (free) | Runways, pavements, painted lines, stands, taxi routing network, signs, PAPI/VASI, helipads, frequencies, boundary |
| OpenStreetMap | Overpass (one bbox query per airport) | ODbL | Buildings, terminals, towers, fences, service roads, water, construction, bridges, deicing, apron classification, stand names; full fallback for pavement/lines/stands |
| OurAirports | CSV download | Public domain | Airport index, ICAO/IATA, ARP, elevation, runway cross-check |
| FAA NASR (US only) | 28-day CSV zip | Public domain | Arresting systems, LAHSO |
| User overrides | `overrides/<ICAO>/<layer>.geojson` | n/a | Any layer, merged last (fills Hotspot, ATCBlindSpot, survey data where no free source exists) |

## Output contract

```
out/
  index.json                      # all airports built, bbox, sources used, generated time
  <ICAO>/
    manifest.json                 # arp, elevation, per-layer feature counts, source per layer, warnings
    <layer>.geojson               # 45 files, always present (empty FeatureCollection when no data)
    <layer>.pbf                   # same content, Geobuf encoding
```

Layer file names are the lowercase AMXM feature names (e.g. `runwayelement`,
`taxiwayguidanceline`, `asrnnode`). Properties use the DO-272 attribute
abbreviations (`idarpt`, `idrwy`, `idthr`, `idlin`, `idstd`, `surftype`, `pcn`,
`status`, `feattype`, `edgetype`, `nodetype`, `catstop`, `source`, ...). Every
feature has a stable `id` (`<ICAO>:<layer>:<n>` assigned in canonical order).

All 45 layers: ATCBlindSpot, AerodromeReferencePoint, AerodromeSign,
AerodromeSurfaceLighting, ApronElement, ArrestingGearLocation,
ArrestingSystemLocation, AsrnEdge, AsrnNode, Blastpad, BridgeSide,
ConstructionArea, DeicingArea, DeicingGroup, FinalApproachAndTakeOffArea,
FrequencyArea, HelipadThreshold, Hotspot, LandAndHoldShortOperationLocation,
PaintedCenterline, ParkingStandArea, ParkingStandLocation, PositionMarking,
RunwayCenterlinePoint, RunwayDisplacedArea, RunwayElement, RunwayExitLine,
RunwayIntersection, RunwayMarking, RunwayShoulder, RunwayThreshold, ServiceRoad,
StandGuidanceLine, Stopway, SurveyControlPoint, TaxiwayElement,
TaxiwayGuidanceLine, TaxiwayHoldingPosition, TaxiwayIntersectionMarking,
TaxiwayShoulder, TouchDownLiftOffArea, VerticalLineStructure,
VerticalPointStructure, VerticalPolygonalStructure, Water.

## Pipeline

1. **Resolve airport**: ICAO -> OurAirports row (ARP, elevation, IATA, name).
2. **Fetch** (no cache unless `--cache DIR`): Gateway recommended scenery
   apt.dat (or a local X-Plane install), Overpass JSON for the airport bbox,
   NASR CSVs for US airports.
3. **Parse** each source into a common `SourceAirport` intermediate model:
   runways, pavements, painted lines, stands, routing nodes/edges, signs,
   lights, helipads, buildings, point/line structures, roads, water,
   construction, bridges, deicing, frequencies, boundary.
4. **Project** everything into a local AEQD metre frame centred on the ARP so
   buffers, intersections, and distances are plain planar maths.
5. **Conflate and derive** into AMDB layers:
   - apt.dat wins for pavement, lines, stands, ASRN; OSM fills gaps and supplies
     buildings/roads/water/etc.; if the Gateway has no scenery, OSM supplies all.
   - Runway rectangles from end coordinates; thresholds, displaced areas,
     blastpads, shoulders, painted centreline, centreline points derived.
   - Runway markings generated per ICAO Annex 14 dimensions from the marking code.
   - Runway intersections via polygon intersection; runway elements have them
     subtracted so layers do not overlap.
   - Taxiway vs apron classification by description keywords, stand containment,
     routing-edge containment, and OSM apron overlap.
   - Painted line codes map to guidance lines, holding positions (CAT I / II-III),
     intersection markings, stand lead-in lines, roadway lines.
   - Runway exit lines = guidance-line portions inside runway elements.
   - ASRN: apt.dat routing network, extended with stand nodes/edges; falls back
     to a graph built from OSM taxiway centrelines.
   - Stand areas from size codes (wingspan class rectangles) or OSM areas.
   - Frequency areas: one aerodrome-boundary polygon per GND/TWR frequency.
   - Taxiway shoulders derived by buffering (marked `source=derived`).
6. **Validate**: valid rings, unique ids, ASRN edge endpoints exist, thresholds
   lie on runway elements, no NaN coordinates.
7. **Write** GeoJSON + Geobuf, manifest, index.

## Code layout (single crate `amdbgen`)

```
src/
  main.rs, cli.rs
  model/      feature structs, enums (code lists), layer registry, ids
  geom/       local projection, buffer, boolean ops helpers, bezier tessellation
  sources/    xplane/{gateway,local,aptdat}.rs, osm/{overpass,elements,tags}.rs, index.rs, faa.rs, overrides.rs
  ir.rs       SourceAirport intermediate model
  build/      classify.rs, runway.rs, taxiway.rs, stands.rs, asrn.rs, marking.rs, structures.rs, conflate.rs
  output/     geojson.rs, geobuf.rs, manifest.rs
  validate.rs
  cache.rs
```

## Error handling

Per-airport failures are recorded in the manifest and index and never abort a
batch. Network calls retry with backoff; the Gateway and Overpass are throttled.
Missing source data is a warning, never an error: layers are always written.

## Testing

Fixture-based unit tests for the apt.dat parser, Overpass JSON parser, NASR CSV
reader; property tests for projection round-trip and buffers;
a synthetic fixture airport as a golden test over all 45 layers; Geobuf
round-trip against the GeoJSON writer. Tests never touch the network.
