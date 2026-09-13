//! GeoJSON (RFC 7946) writer/reader for AMDB features.

use super::Projection;
use crate::geom::LocalFrame;
use crate::model::{AmdbFeature, Layer, Props};
use anyhow::{anyhow, Result};
use geo_types::{Coord, Geometry, LineString, MultiLineString, MultiPoint, MultiPolygon, Point, Polygon};
use serde_json::{json, Map, Value};

fn round(v: f64, decimals: i32) -> f64 {
    let m = 10f64.powi(decimals);
    (v * m).round() / m
}

fn coord(c: &Coord<f64>, d: i32) -> Value {
    json!([round(c.x, d), round(c.y, d)])
}

fn line(l: &LineString<f64>, d: i32) -> Value {
    Value::Array(l.0.iter().map(|c| coord(c, d)).collect())
}

fn poly(p: &Polygon<f64>, d: i32) -> Value {
    let mut rings = vec![line(p.exterior(), d)];
    rings.extend(p.interiors().iter().map(|r| line(r, d)));
    Value::Array(rings)
}

pub fn geometry_to_json(g: &Geometry<f64>, decimals: i32) -> Value {
    match g {
        Geometry::Point(p) => json!({"type":"Point","coordinates": coord(&p.0, decimals)}),
        Geometry::LineString(l) => json!({"type":"LineString","coordinates": line(l, decimals)}),
        Geometry::Polygon(p) => json!({"type":"Polygon","coordinates": poly(p, decimals)}),
        Geometry::MultiPoint(m) => json!({"type":"MultiPoint","coordinates": Value::Array(m.0.iter().map(|p| coord(&p.0, decimals)).collect())}),
        Geometry::MultiLineString(m) => json!({"type":"MultiLineString","coordinates": Value::Array(m.0.iter().map(|l| line(l, decimals)).collect())}),
        Geometry::MultiPolygon(m) => json!({"type":"MultiPolygon","coordinates": Value::Array(m.0.iter().map(|p| poly(p, decimals)).collect())}),
        Geometry::Line(l) => json!({"type":"LineString","coordinates": [coord(&l.start, decimals), coord(&l.end, decimals)]}),
        Geometry::Rect(r) => json!({"type":"Polygon","coordinates": poly(&r.to_polygon(), decimals)}),
        Geometry::Triangle(t) => json!({"type":"Polygon","coordinates": poly(&t.to_polygon(), decimals)}),
        Geometry::GeometryCollection(gc) => json!({"type":"GeometryCollection","geometries": Value::Array(gc.0.iter().map(|g| geometry_to_json(g, decimals)).collect())}),
    }
}

pub fn feature_to_json(f: &AmdbFeature, decimals: i32) -> Value {
    let mut o = Map::new();
    o.insert("type".into(), "Feature".into());
    if let Some(id) = f.props.get("id") {
        o.insert("id".into(), id.clone());
    }
    o.insert("geometry".into(), geometry_to_json(&f.geom, decimals));
    o.insert("properties".into(), Value::Object(f.props.clone()));
    Value::Object(o)
}

pub fn decimals_for(p: Projection) -> i32 {
    match p {
        Projection::Wgs84 => 7,
        Projection::LocalMetres => 3,
    }
}

/// Serialise one layer as a FeatureCollection with foreign members describing the
/// airport, layer and projection.
pub fn feature_collection_string(icao: &str, layer: Layer, feats: &[AmdbFeature], projection: Projection, frame: &LocalFrame) -> String {
    let d = decimals_for(projection);
    let fc = json!({
        "type": "FeatureCollection",
        "name": layer.name(),
        "amdb": {
            "icao": icao,
            "layer": layer.name(),
            "geometry": format!("{:?}", layer.kind()).to_lowercase(),
            "projection": projection.name(),
            "arp": {"lat": frame.lat0, "lon": frame.lon0},
            "count": feats.len(),
        },
        "features": Value::Array(feats.iter().map(|f| feature_to_json(f, d)).collect()),
    });
    serde_json::to_string(&fc).expect("json")
}

fn parse_coord(v: &Value) -> Result<Coord<f64>> {
    let a = v.as_array().ok_or_else(|| anyhow!("coordinate not an array"))?;
    if a.len() < 2 {
        return Err(anyhow!("coordinate needs 2 values"));
    }
    Ok(Coord { x: a[0].as_f64().ok_or_else(|| anyhow!("x"))?, y: a[1].as_f64().ok_or_else(|| anyhow!("y"))? })
}

fn parse_line(v: &Value) -> Result<LineString<f64>> {
    Ok(LineString(v.as_array().ok_or_else(|| anyhow!("line not an array"))?.iter().map(parse_coord).collect::<Result<Vec<_>>>()?))
}

fn parse_poly(v: &Value) -> Result<Polygon<f64>> {
    let rings: Vec<LineString<f64>> = v.as_array().ok_or_else(|| anyhow!("polygon not an array"))?.iter().map(parse_line).collect::<Result<_>>()?;
    let mut it = rings.into_iter();
    let ext = it.next().ok_or_else(|| anyhow!("polygon without exterior"))?;
    Ok(Polygon::new(ext, it.collect()))
}

pub fn geometry_from_json(v: &Value) -> Result<Geometry<f64>> {
    let ty = v.get("type").and_then(Value::as_str).ok_or_else(|| anyhow!("geometry without type"))?;
    let c = || v.get("coordinates").ok_or_else(|| anyhow!("geometry without coordinates"));
    Ok(match ty {
        "Point" => Geometry::Point(Point(parse_coord(c()?)?)),
        "LineString" => Geometry::LineString(parse_line(c()?)?),
        "Polygon" => Geometry::Polygon(parse_poly(c()?)?),
        "MultiPoint" => Geometry::MultiPoint(MultiPoint(c()?.as_array().ok_or_else(|| anyhow!("mp"))?.iter().map(|p| parse_coord(p).map(Point)).collect::<Result<_>>()?)),
        "MultiLineString" => Geometry::MultiLineString(MultiLineString(c()?.as_array().ok_or_else(|| anyhow!("mls"))?.iter().map(parse_line).collect::<Result<_>>()?)),
        "MultiPolygon" => Geometry::MultiPolygon(MultiPolygon(c()?.as_array().ok_or_else(|| anyhow!("mpoly"))?.iter().map(parse_poly).collect::<Result<_>>()?)),
        other => return Err(anyhow!("unsupported geometry type {other}")),
    })
}

/// Parse a FeatureCollection into features of `layer`. Returns (replace flag, features).
pub fn parse_feature_collection(text: &str, layer: Layer) -> Result<(bool, Vec<AmdbFeature>)> {
    let v: Value = serde_json::from_str(text)?;
    let replace = v.get("replace").and_then(Value::as_bool).unwrap_or(false);
    let feats = v.get("features").and_then(Value::as_array).ok_or_else(|| anyhow!("no features array"))?;
    let mut out = Vec::new();
    for f in feats {
        let Some(g) = f.get("geometry") else { continue };
        let geom = geometry_from_json(g)?;
        let mut props: Props = f.get("properties").and_then(Value::as_object).cloned().unwrap_or_default();
        if let Some(id) = f.get("id") {
            props.entry("id".to_string()).or_insert(id.clone());
        }
        out.push(AmdbFeature { layer, geom, props });
    }
    Ok((replace, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_polygon_feature() {
        let p = Polygon::new(LineString(vec![Coord { x: 0.0, y: 0.0 }, Coord { x: 1.0, y: 0.0 }, Coord { x: 1.0, y: 1.0 }, Coord { x: 0.0, y: 0.0 }]), vec![]);
        let f = AmdbFeature::new(Layer::ApronElement, p).with("id", "X:apronelement:1").with("surftype", 2);
        let frame = LocalFrame::new(0.0, 0.0);
        let s = feature_collection_string("X", Layer::ApronElement, &[f], Projection::Wgs84, &frame);
        assert!(s.contains("\"name\":\"apronelement\""));
        let (replace, feats) = parse_feature_collection(&s, Layer::ApronElement).unwrap();
        assert!(!replace);
        assert_eq!(feats.len(), 1);
        assert_eq!(feats[0].get_str("id"), Some("X:apronelement:1"));
        assert!(matches!(feats[0].geom, Geometry::Polygon(_)));
    }
}
