//! HTTP server implementing the Navigraph AMDB API surface exactly as the official
//! `@navigraph/amdb` SDK calls it:
//!
//! * `GET /v1/cycle`
//! * `GET /v1/search?q=`                                  (prefix match on idarpt, iata, name)
//! * `GET /v1/{ICAO}?include=a,b&exclude=c&projection=&precision=`
//! * `GET /v1/{ICAO}/{layer}?projection=&precision=`
//!
//! Any path prefix before `/v1/` is ignored so SDK-style hosts also land here.
//! Authorization headers are accepted and ignored.

use super::compat::{feattype, layer_from_client_name, NAVIGRAPH_LAYERS};
use super::store::{AirportData, Store};
use crate::geom::ops;
use crate::model::{AmdbFeature, Layer};
use crate::output::geojson::geometry_to_json;
use anyhow::Result;
use geo::Centroid;
use geo_types::{Coord, Geometry, LineString, Point};
use serde_json::{json, Map, Value};
use std::sync::Arc;
use tiny_http::{Header, Method, Request, Response, Server};

fn header(k: &str, v: &str) -> Header {
    Header::from_bytes(k.as_bytes(), v.as_bytes()).expect("header")
}

fn respond_json(req: Request, status: u16, body: String) {
    let mut resp = Response::from_string(body).with_status_code(status);
    resp.add_header(header("Content-Type", "application/json; charset=utf-8"));
    resp.add_header(header("Access-Control-Allow-Origin", "*"));
    resp.add_header(header("Access-Control-Allow-Headers", "*"));
    resp.add_header(header("Cache-Control", "no-store"));
    let _ = req.respond(resp);
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() + 1 && i + 2 <= bytes.len() - 1 => {
                let h = std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
                if let Some(v) = h {
                    out.push(v);
                    i += 3;
                    continue;
                }
                out.push(b'%');
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_query(q: &str) -> Map<String, Value> {
    let mut m = Map::new();
    for kv in q.split('&').filter(|s| !s.is_empty()) {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        m.insert(url_decode(k), Value::from(url_decode(v)));
    }
    m
}

/// Current AIRAC-style cycle (28-day cycles from a known start).
pub fn cycle_json() -> Value {
    let epoch = chrono::NaiveDate::from_ymd_opt(2026, 9, 3).unwrap(); // AIRAC 2609 start
    let today = chrono::Utc::now().date_naive();
    let n = (today - epoch).num_days().div_euclid(28);
    let start = epoch + chrono::Duration::days(n * 28);
    let end = start + chrono::Duration::days(28);
    let year: i32 = start.format("%Y").to_string().parse().unwrap();
    let jan = chrono::NaiveDate::from_ymd_opt(year, 1, 1).unwrap();
    let first_in_year = {
        let k = (jan - epoch).num_days().div_euclid(28);
        let mut s = epoch + chrono::Duration::days(k * 28);
        if s < jan {
            s += chrono::Duration::days(28);
        }
        s
    };
    let idx = (start - first_in_year).num_days() / 28 + 1;
    json!({
        "cycle_start_date": start.to_string(),
        "cycle_end_date": end.to_string(),
        "import_time": start.to_string(),
        "airac_cycle": format!("{}{idx:02}", start.format("%y")).parse::<i64>().unwrap_or(0),
    })
}

fn snap(c: Coord<f64>, precision: Option<f64>) -> Coord<f64> {
    match precision {
        Some(p) if p > 0.0 => Coord { x: (c.x / p).round() * p, y: (c.y / p).round() * p },
        _ => c,
    }
}

fn snap_geometry(g: &Geometry<f64>, precision: Option<f64>) -> Geometry<f64> {
    let Some(_) = precision else { return g.clone() };
    let ls = |l: &LineString<f64>| LineString(l.0.iter().map(|c| snap(*c, precision)).collect());
    match g {
        Geometry::Point(p) => Geometry::Point(Point(snap(p.0, precision))),
        Geometry::LineString(l) => Geometry::LineString(ls(l)),
        Geometry::Polygon(p) => Geometry::Polygon(geo_types::Polygon::new(ls(p.exterior()), p.interiors().iter().map(ls).collect())),
        Geometry::MultiPolygon(m) => Geometry::MultiPolygon(geo_types::MultiPolygon(m.0.iter().map(|p| geo_types::Polygon::new(ls(p.exterior()), p.interiors().iter().map(ls).collect())).collect())),
        Geometry::MultiLineString(m) => Geometry::MultiLineString(geo_types::MultiLineString(m.0.iter().map(ls).collect())),
        other => other.clone(),
    }
}

/// Geometry-derived members Navigraph adds: `centroid` for polygons, `midpoint` and
/// `longest_segment` for lines, both in the response projection.
fn derived_props(geom: &Geometry<f64>, decimals: i32) -> Vec<(&'static str, Value)> {
    let mut out = Vec::new();
    match geom {
        Geometry::Polygon(_) | Geometry::MultiPolygon(_) => {
            if let Some(c) = geom.centroid() {
                out.push(("centroid", geometry_to_json(&Geometry::Point(c), decimals)));
            }
        }
        Geometry::LineString(l) => {
            let len = ops::length(l);
            let mid = ops::point_at(l, len / 2.0);
            out.push(("midpoint", geometry_to_json(&Geometry::Point(Point(mid)), decimals)));
            let longest = l.0.windows(2).map(|w| ops::dist(w[0], w[1])).fold(0.0, f64::max);
            out.push(("longest_segment", Value::from((longest * 100.0).round() / 100.0)));
        }
        Geometry::MultiLineString(m) => {
            if let Some(l) = m.0.iter().max_by(|a, b| ops::length(a).partial_cmp(&ops::length(b)).unwrap()) {
                let len = ops::length(l);
                out.push(("midpoint", geometry_to_json(&Geometry::Point(Point(ops::point_at(l, len / 2.0))), decimals)));
                let longest = l.0.windows(2).map(|w| ops::dist(w[0], w[1])).fold(0.0, f64::max);
                out.push(("longest_segment", Value::from((longest * 100.0).round() / 100.0)));
            }
        }
        _ => {}
    }
    out
}

/// One layer as a FeatureCollection in the requested projection.
fn layer_collection(ap: &AirportData, layer: Layer, wgs84: bool, precision: Option<f64>) -> Value {
    let decimals = if wgs84 { 7 } else { 2 };
    let feats = ap.layers.get(&layer).map(Vec::as_slice).unwrap_or(&[]);
    let arr: Vec<Value> = feats
        .iter()
        .map(|f: &AmdbFeature| {
            let mut geom = if wgs84 { ap.frame.to_wgs84(&f.geom) } else { f.geom.clone() };
            geom = snap_geometry(&geom, precision);
            let mut props = f.props.clone();
            for (k, v) in derived_props(&geom, decimals) {
                props.insert(k.to_string(), v);
            }
            json!({"type":"Feature","geometry": geometry_to_json(&geom, decimals),"properties": Value::Object(props)})
        })
        .collect();
    json!({"type":"FeatureCollection","features": arr})
}

fn handle(store: &Store, req: Request) {
    let url = req.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((&url, ""));
    let params = parse_query(query);
    if *req.method() == Method::Options {
        respond_json(req, 204, String::new());
        return;
    }
    let Some(pos) = path.find("/v1/") else {
        respond_json(req, 404, json!({"error":"not found","hint":"expected /v1/..."}).to_string());
        return;
    };
    let rest = path[pos + 4..].trim_matches('/');
    let t0 = std::time::Instant::now();
    let projection = params.get("projection").and_then(Value::as_str).unwrap_or("NAVIGRAPH:ARP_AZEQ").to_string();
    let wgs84 = projection.eq_ignore_ascii_case("EPSG:4326");
    let precision = params.get("precision").and_then(Value::as_str).and_then(|s| s.parse::<f64>().ok());
    let mut parts = rest.splitn(2, '/');
    let head = parts.next().unwrap_or("");
    let tail = parts.next();
    match (head, tail) {
        ("cycle", None) => respond_json(req, 200, cycle_json().to_string()),
        ("search", None) => {
            let q = params.get("q").and_then(Value::as_str).unwrap_or("");
            let results = store.search(q);
            crate::term::info(&format!("Search {:?}: {} airports", q, results.len()));
            respond_json(req, 200, Value::Array(results).to_string());
        }
        ("", None) => respond_json(req, 200, json!({"service":"amdb-bridge","version":env!("CARGO_PKG_VERSION"),"loaded":store.loaded_count()}).to_string()),
        (icao, layer_name) if icao.len() == 4 && icao.chars().all(|c| c.is_ascii_alphanumeric()) => {
            let ap = match store.airport(icao) {
                Ok(a) => a,
                Err(e) => {
                    log::error!("{icao}: {e:#}");
                    respond_json(req, 404, json!({"error": format!("{e:#}")}).to_string());
                    return;
                }
            };
            match layer_name {
                Some(name) => {
                    // Single layer: the bare FeatureCollection.
                    match layer_from_client_name(name).filter(|l| feattype(*l).is_some()) {
                        Some(l) => {
                            crate::term::success(&format!("[{icao}] Served {name} in {}", crate::term::human_secs(t0.elapsed().as_secs_f64())));
                            respond_json(req, 200, layer_collection(&ap, l, wgs84, precision).to_string());
                        }
                        None => respond_json(req, 404, json!({"error": format!("unknown layer {name}")}).to_string()),
                    }
                }
                None => {
                    let include: Vec<(String, Layer)> = params.get("include").and_then(Value::as_str).unwrap_or("").split(',').map(str::trim).filter(|s| !s.is_empty()).filter_map(|n| layer_from_client_name(n).map(|l| (n.to_ascii_lowercase(), l))).collect();
                    let exclude: Vec<Layer> = params.get("exclude").and_then(Value::as_str).unwrap_or("").split(',').filter_map(layer_from_client_name).collect();
                    let wanted: Vec<(String, Layer)> = if include.is_empty() {
                        NAVIGRAPH_LAYERS.iter().map(|l| (l.name().to_string(), *l)).collect()
                    } else {
                        include
                    }
                    .into_iter()
                    .filter(|(_, l)| !exclude.contains(l))
                    .collect();
                    let mut body = Map::new();
                    for (name, l) in wanted {
                        body.insert(name, layer_collection(&ap, l, wgs84, precision));
                    }
                    crate::term::success(&format!("[{icao}] Served {} layers ({}) in {}", body.len(), if wgs84 { "lat/lon" } else { "ARP metres" }, crate::term::human_secs(t0.elapsed().as_secs_f64())));
                    respond_json(req, 200, Value::Object(body).to_string());
                }
            }
        }
        _ => respond_json(req, 404, json!({"error":"not found","path":rest}).to_string()),
    }
}

/// Listeners to run. HTTPS answers the redirected Navigraph host; HTTP is for local
/// tools and patched bundles. Both bind to loopback only.
pub struct Listen {
    pub http_port: Option<u16>,
    /// (port, certificate PEM, private key PEM)
    pub https: Option<(u16, Vec<u8>, Vec<u8>)>,
}

/// Run the server until the process exits.
pub fn serve(store: Store, listen: Listen) -> Result<()> {
    let store = Arc::new(store);
    let mut handles = Vec::new();
    if let Some((port, cert, key)) = listen.https {
        let addr = format!("127.0.0.1:{port}");
        let server = Server::https(&addr, tiny_http::SslConfig { certificate: cert, private_key: key }).map_err(|e| anyhow::anyhow!("bind https {addr}: {e}"))?;
        crate::term::success(&format!("Listening on https://{addr}/v1/  (airports in {})", store.out.display()));
        let st = store.clone();
        handles.push(std::thread::spawn(move || {
            for req in server.incoming_requests() {
                let st = st.clone();
                std::thread::spawn(move || handle(&st, req));
            }
        }));
    }
    if let Some(port) = listen.http_port {
        let addr = format!("127.0.0.1:{port}");
        let server = Server::http(&addr).map_err(|e| anyhow::anyhow!("bind http {addr}: {e}"))?;
        crate::term::info(&format!("Also on http://{addr}/v1/ for local tools"));
        let st = store.clone();
        handles.push(std::thread::spawn(move || {
            for req in server.incoming_requests() {
                let st = st.clone();
                std::thread::spawn(move || handle(&st, req));
            }
        }));
    }
    if handles.is_empty() {
        return Err(anyhow::anyhow!("nothing to listen on"));
    }
    for h in handles {
        let _ = h.join();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_query_and_snaps() {
        let m = parse_query("projection=NAVIGRAPH%3AARP_AZEQ&include=a%2Cb&q=");
        assert_eq!(m["projection"], "NAVIGRAPH:ARP_AZEQ");
        assert_eq!(m["include"], "a,b");
        assert_eq!(m["q"], "");
        let c = cycle_json();
        assert_eq!(c["cycle_start_date"].as_str().unwrap().len(), 10);
        let s = snap(Coord { x: 12.34, y: -7.77 }, Some(0.5));
        assert_eq!((s.x, s.y), (12.5, -8.0));
        let d = derived_props(&Geometry::LineString(LineString(vec![Coord { x: 0.0, y: 0.0 }, Coord { x: 10.0, y: 0.0 }, Coord { x: 10.0, y: 4.0 }])), 2);
        assert_eq!(d[1].1, 10.0);
        assert_eq!(d[0].1["coordinates"], json!([7.0, 0.0]));
    }
}
