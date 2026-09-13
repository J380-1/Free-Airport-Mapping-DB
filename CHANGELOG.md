# Changelog

## 0.1.0 (unreleased)

- `amdbgen`: builds all 45 DO-272 / AMXM airport-mapping layers for any airport from
  the X-Plane Scenery Gateway, OpenStreetMap, OurAirports and (US only) FAA NASR, as
  GeoJSON and Geobuf PBF, in WGS84 or ARP-centred metres.
- `amdb-bridge`: serves the data through the Navigraph AMDB API surface (1:1 with the
  official `@navigraph/amdb` SDK schema) so aircraft OANS / ANF / BTV work unchanged;
  hosts-file redirect with local TLS, cleaned up on exit; bundle patcher as an
  alternative.
- SimBrief import (`--simbrief`) to build or prefetch a flight's airports.
- OANS-style and Jeppesen-style HTML previews (`tools/`).
