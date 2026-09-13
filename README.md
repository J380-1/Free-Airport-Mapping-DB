# Free Airport Mapping DB

[![CI](https://github.com/Vihaan2012-cmyk/Free-Airport-Mapping-DB/actions/workflows/ci.yml/badge.svg)](https://github.com/Vihaan2012-cmyk/Free-Airport-Mapping-DB/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](https://www.rust-lang.org)
[![DO-272 layers](https://img.shields.io/badge/DO--272%20layers-45-success)](#output)
[![Navigraph AMDB API](https://img.shields.io/badge/API-Navigraph%20AMDB%20compatible-8A2BE2)](#aircraft-bridge-amdb-bridge)
[![MSFS](https://img.shields.io/badge/MSFS-2020%20%7C%202024-informational)](#aircraft-bridge-amdb-bridge)
[![Data](https://img.shields.io/badge/sources-X--Plane%20Gateway%20%2B%20OpenStreetMap-lightgrey)](#sources)

A free, worldwide **Airport Mapping Database (AMDB)** for flight simulation: every
DO-272 / AMXM layer for any airport, built from open sources, served to aircraft in
exactly the shape their OANS / ANF / BTV code already expects from Navigraph.

Two binaries:

- **`amdbgen`** builds the data: 45 layers per airport as GeoJSON and Geobuf PBF.
- **`amdb-bridge`** serves it through the Navigraph AMDB API surface so aircraft like
  the FlyByWire A380X show it without any code changes.

![amdbgen building Frankfurt](docs/cli-a3357c7.svg)

## Quick start

```
cargo build --release
target\release\amdbgen build EDDF KJFK               # any ICAO, worldwide
target\release\amdbgen build --simbrief YOUR_NAME    # your latest SimBrief OFP airports
target\release\amdb-bridge serve --out out           # feed the aircraft (see below)
```

## Sources

All free, no keys, nothing to install.

| Source | Used for | Licence |
|---|---|---|
| X-Plane Scenery Gateway | Runways, pavement, painted lines, stands, taxi routing network, signs, lights, frequencies | free |
| OpenStreetMap (map API, tiled) | Terminals, buildings, towers, fences, roads, water, construction, deicing | ODbL |
| OurAirports | Worldwide airport index | public domain |
| FAA NASR (US only, automatic) | Declared distances, stopways, arresting systems, LAHSO | public domain |
| Local X-Plane install (optional) | Your own scenery instead of the Gateway: `--xplane-dir`, `--aptdat` | yours |
| `overrides/<ICAO>/<layer>.geojson` | Anything to add or replace per airport (hotspots, blind spots) | yours |

The two index files are cached under `%LOCALAPPDATA%\amdbgen\index` and refreshed
daily. Nothing else is cached unless you pass `--cache DIR`.

## Output

```
out/
  index.json               every airport built
  codes.json               legend for the numeric attribute codes
  EDDF/
    manifest.json          ARP, elevation, per-layer counts, sources, empty reasons
    runwayelement.geojson  one FeatureCollection per layer ...
    runwayelement.pbf      ... and the same as Geobuf
    ...                    45 layers
```

Properties use the DO-272 names (`idarpt`, `idrwy`, `idthr`, `idlin`, `idstd`,
`surftype`, `catstop`, `tora/toda/asda/lda`, ...). Layers with no free worldwide
source (hotspot, ATC blind spot, survey points) are written empty with the reason in
the manifest and filled from `overrides/`.

Flags: `--profile full|map`, `--layers a,b,c`, `--format geojson,pbf`,
`--projection wgs84|metres`, `--country IN`, `--icao-prefix ED`, `--all`,
`--osm osmapi|overpass|off`, `--faa off`, `-v`.

## Aircraft bridge (`amdb-bridge`)

Any aircraft that talks to `amdb.api.navigraph.com` gets this data instead, with the
same features it has with Navigraph (map, labels, BTV, FMS runway highlight), because
the aircraft code is untouched: only the address changes.

```
amdb-bridge serve --out out                 # redirect + serve; keep it running while the sim is up
amdb-bridge prefetch EDDF KJFK              # or: --simbrief YOUR_NAME
amdb-bridge status                          # redirect / certificate / detected aircraft
amdb-bridge cleanup                         # remove redirect + certificate
amdb-bridge autostart --out out             # start with the sim (exe.xml)
```

`serve` asks for administrator rights, points `amdb.api.navigraph.com` at your PC
through the hosts file, answers over HTTPS with a locally generated certificate it
registers as trusted, and removes the redirect when it stops. The API and response
schema follow Navigraph's own SDK (`@navigraph/amdb`) 1:1: `/v1/cycle`,
`/v1/search?q=`, `/v1/{ICAO}?include=&exclude=&projection=&precision=`,
`/v1/{ICAO}/{layer}`, all 36 Navigraph layers with their exact property sets and
enum values. No Navigraph account is needed. `patch` / `unpatch` are a no-admin
alternative that rewrites the aircraft bundles instead.

## Previews

```
python tools/oans_view.py out/EDDF viewer.html    # OANS-style moving map
python tools/jepp_chart.py out/KJFK chart.html    # Jeppesen-style airport diagram
```

## Layout

- `src/model` layer registry, code lists, feature container
- `src/geom` local metre projection, Bézier tessellation, buffers, boolean ops
- `src/sources` xplane (gateway, local, apt.dat), osm (map API, Overpass, tags), index, faa, simbrief, overrides
- `src/build` conflation and derivation: runways, markings, pavement, lines, stands, ASRN, structures, signs
- `src/output` GeoJSON, Geobuf, manifest
- `src/bridge` server, Navigraph-schema compat, hosts redirect, TLS, patcher
- `src/pipeline.rs` fetch, merge, build, validate, write

## Licence

MIT. Generated data derives from OpenStreetMap (ODbL) and the X-Plane Scenery
Gateway; it is for simulation only and not for real-world navigation.
