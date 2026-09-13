//! Geobuf (Mapbox protobuf encoding of GeoJSON) writer, so every layer is also
//! available as a compact `.pbf` readable by any geobuf decoder.

use super::Projection;
use crate::model::AmdbFeature;
use geo_types::{Coord, Geometry, LineString, Polygon};
use prost::Message;
use serde_json::Value;
use std::collections::HashMap;

pub mod proto {
    use prost::{Message, Oneof};

    #[derive(Clone, PartialEq, Message)]
    pub struct Data {
        #[prost(string, repeated, tag = "1")]
        pub keys: Vec<String>,
        #[prost(uint32, optional, tag = "2")]
        pub dimensions: Option<u32>,
        #[prost(uint32, optional, tag = "3")]
        pub precision: Option<u32>,
        #[prost(oneof = "DataType", tags = "4, 5, 6")]
        pub data_type: Option<DataType>,
    }

    #[derive(Clone, PartialEq, Oneof)]
    pub enum DataType {
        #[prost(message, tag = "4")]
        FeatureCollection(FeatureCollection),
        #[prost(message, tag = "5")]
        Feature(Feature),
        #[prost(message, tag = "6")]
        Geometry(Geometry),
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct FeatureCollection {
        #[prost(message, repeated, tag = "1")]
        pub features: Vec<Feature>,
        #[prost(message, repeated, tag = "13")]
        pub values: Vec<Value>,
        #[prost(uint32, repeated, tag = "15")]
        pub custom_properties: Vec<u32>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct Feature {
        #[prost(message, optional, tag = "1")]
        pub geometry: Option<Geometry>,
        #[prost(oneof = "IdType", tags = "11, 12")]
        pub id_type: Option<IdType>,
        #[prost(message, repeated, tag = "13")]
        pub values: Vec<Value>,
        #[prost(uint32, repeated, tag = "14")]
        pub properties: Vec<u32>,
        #[prost(uint32, repeated, tag = "15")]
        pub custom_properties: Vec<u32>,
    }

    #[derive(Clone, PartialEq, Oneof)]
    pub enum IdType {
        #[prost(string, tag = "11")]
        Id(String),
        #[prost(sint64, tag = "12")]
        IntId(i64),
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct Geometry {
        #[prost(int32, tag = "1")]
        pub r#type: i32,
        #[prost(uint32, repeated, tag = "2")]
        pub lengths: Vec<u32>,
        #[prost(sint64, repeated, tag = "3")]
        pub coords: Vec<i64>,
        #[prost(message, repeated, tag = "4")]
        pub geometries: Vec<Geometry>,
        #[prost(message, repeated, tag = "13")]
        pub values: Vec<Value>,
        #[prost(uint32, repeated, tag = "15")]
        pub custom_properties: Vec<u32>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct Value {
        #[prost(oneof = "ValueType", tags = "1, 2, 3, 4, 5, 6")]
        pub value_type: Option<ValueType>,
    }

    #[derive(Clone, PartialEq, Oneof)]
    pub enum ValueType {
        #[prost(string, tag = "1")]
        StringValue(String),
        #[prost(double, tag = "2")]
        DoubleValue(f64),
        #[prost(uint64, tag = "3")]
        PosIntValue(u64),
        #[prost(uint64, tag = "4")]
        NegIntValue(u64),
        #[prost(bool, tag = "5")]
        BoolValue(bool),
        #[prost(string, tag = "6")]
        JsonValue(String),
    }

    pub const POINT: i32 = 0;
    pub const MULTIPOINT: i32 = 1;
    pub const LINESTRING: i32 = 2;
    pub const MULTILINESTRING: i32 = 3;
    pub const POLYGON: i32 = 4;
    pub const MULTIPOLYGON: i32 = 5;
    pub const GEOMETRYCOLLECTION: i32 = 6;
}

pub fn precision_for(p: Projection) -> u32 {
    match p {
        Projection::Wgs84 => 7,
        Projection::LocalMetres => 3,
    }
}

struct Enc {
    e: f64,
}

impl Enc {
    fn line(&self, out: &mut Vec<i64>, pts: &[Coord<f64>], closed: bool) {
        let n = if closed { pts.len().saturating_sub(1) } else { pts.len() };
        let (mut sx, mut sy) = (0i64, 0i64);
        for p in &pts[..n] {
            let x = (p.x * self.e).round() as i64 - sx;
            let y = (p.y * self.e).round() as i64 - sy;
            out.push(x);
            out.push(y);
            sx += x;
            sy += y;
        }
    }

    fn multi_line(&self, g: &mut proto::Geometry, lines: &[&LineString<f64>], closed: bool) {
        if lines.len() != 1 {
            g.lengths = lines.iter().map(|l| (l.0.len() - usize::from(closed)) as u32).collect();
        }
        for l in lines {
            self.line(&mut g.coords, &l.0, closed);
        }
    }

    fn multi_polygon(&self, g: &mut proto::Geometry, polys: &[&Polygon<f64>]) {
        let rings = |p: &Polygon<f64>| -> Vec<LineString<f64>> {
            let mut v = vec![p.exterior().clone()];
            v.extend(p.interiors().iter().cloned());
            v
        };
        let all: Vec<Vec<LineString<f64>>> = polys.iter().map(|p| rings(p)).collect();
        if all.len() != 1 || all[0].len() != 1 {
            let mut lengths = vec![all.len() as u32];
            for p in &all {
                lengths.push(p.len() as u32);
                for r in p {
                    lengths.push((r.0.len() - 1) as u32);
                }
            }
            g.lengths = lengths;
        }
        for p in &all {
            for r in p {
                self.line(&mut g.coords, &r.0, true);
            }
        }
    }

    fn geometry(&self, geom: &Geometry<f64>) -> proto::Geometry {
        let mut g = proto::Geometry::default();
        match geom {
            Geometry::Point(p) => {
                g.r#type = proto::POINT;
                self.line(&mut g.coords, &[p.0], false);
            }
            Geometry::MultiPoint(m) => {
                g.r#type = proto::MULTIPOINT;
                let pts: Vec<Coord<f64>> = m.0.iter().map(|p| p.0).collect();
                self.line(&mut g.coords, &pts, false);
            }
            Geometry::LineString(l) => {
                g.r#type = proto::LINESTRING;
                self.line(&mut g.coords, &l.0, false);
            }
            Geometry::Line(l) => {
                g.r#type = proto::LINESTRING;
                self.line(&mut g.coords, &[l.start, l.end], false);
            }
            Geometry::MultiLineString(m) => {
                g.r#type = proto::MULTILINESTRING;
                let refs: Vec<&LineString<f64>> = m.0.iter().collect();
                self.multi_line(&mut g, &refs, false);
            }
            Geometry::Polygon(p) => {
                g.r#type = proto::POLYGON;
                let mut rings: Vec<&LineString<f64>> = vec![p.exterior()];
                rings.extend(p.interiors().iter());
                self.multi_line(&mut g, &rings, true);
            }
            Geometry::MultiPolygon(m) => {
                g.r#type = proto::MULTIPOLYGON;
                let refs: Vec<&Polygon<f64>> = m.0.iter().collect();
                self.multi_polygon(&mut g, &refs);
            }
            Geometry::Rect(r) => return self.geometry(&Geometry::Polygon(r.to_polygon())),
            Geometry::Triangle(t) => return self.geometry(&Geometry::Polygon(t.to_polygon())),
            Geometry::GeometryCollection(gc) => {
                g.r#type = proto::GEOMETRYCOLLECTION;
                g.geometries = gc.0.iter().map(|x| self.geometry(x)).collect();
            }
        }
        g
    }
}

fn value(v: &Value) -> proto::Value {
    use proto::ValueType::*;
    let vt = match v {
        Value::String(s) => StringValue(s.clone()),
        Value::Bool(b) => BoolValue(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                if i >= 0 {
                    PosIntValue(i as u64)
                } else {
                    NegIntValue(i.unsigned_abs())
                }
            } else {
                DoubleValue(n.as_f64().unwrap_or(0.0))
            }
        }
        other => JsonValue(other.to_string()),
    };
    proto::Value { value_type: Some(vt) }
}

/// Encode a layer's features as a geobuf FeatureCollection.
pub fn encode(feats: &[AmdbFeature], projection: Projection) -> Vec<u8> {
    let precision = precision_for(projection);
    let enc = Enc { e: 10f64.powi(precision as i32) };
    let mut keys: Vec<String> = Vec::new();
    let mut key_idx: HashMap<String, u32> = HashMap::new();
    let mut fc = proto::FeatureCollection::default();
    for f in feats {
        let mut pf = proto::Feature { geometry: Some(enc.geometry(&f.geom)), ..Default::default() };
        for (k, v) in &f.props {
            if k == "id" {
                pf.id_type = Some(match v {
                    Value::Number(n) if n.is_i64() => proto::IdType::IntId(n.as_i64().unwrap()),
                    Value::String(s) => proto::IdType::Id(s.clone()),
                    other => proto::IdType::Id(other.to_string()),
                });
                continue;
            }
            let ki = *key_idx.entry(k.clone()).or_insert_with(|| {
                keys.push(k.clone());
                (keys.len() - 1) as u32
            });
            pf.values.push(value(v));
            pf.properties.push(ki);
            pf.properties.push((pf.values.len() - 1) as u32);
        }
        fc.features.push(pf);
    }
    let data = proto::Data { keys, dimensions: Some(2), precision: Some(precision), data_type: Some(proto::DataType::FeatureCollection(fc)) };
    data.encode_to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Layer;
    use geo_types::Point;

    #[test]
    fn encodes_and_decodes_feature_collection() {
        let f1 = AmdbFeature::new(Layer::AsrnNode, Point::new(77.1031234, 28.5665678)).with("id", "X:asrnnode:1").with("nodetype", 2).with("name", "A1");
        let poly = Polygon::new(LineString(vec![Coord { x: 0.0, y: 0.0 }, Coord { x: 0.001, y: 0.0 }, Coord { x: 0.001, y: 0.001 }, Coord { x: 0.0, y: 0.0 }]), vec![]);
        let f2 = AmdbFeature::new(Layer::ApronElement, poly).with("id", 7).with("surftype", -3).with("x", 1.5).with("flag", true).with("nul", Value::Null);
        let bytes = encode(&[f1, f2], Projection::Wgs84);
        let d = proto::Data::decode(bytes.as_slice()).unwrap();
        assert_eq!(d.precision, Some(7));
        // Properties are stored sorted by key (serde_json Map), so keys follow that order.
        assert_eq!(d.keys, vec!["name", "nodetype", "flag", "nul", "surftype", "x"]);
        let Some(proto::DataType::FeatureCollection(fc)) = d.data_type else { panic!() };
        assert_eq!(fc.features.len(), 2);
        let g = fc.features[0].geometry.as_ref().unwrap();
        assert_eq!(g.r#type, proto::POINT);
        assert_eq!(g.coords, vec![771031234, 285665678]);
        assert_eq!(fc.features[0].properties, vec![0, 0, 1, 1]);
        assert_eq!(fc.features[0].id_type, Some(proto::IdType::Id("X:asrnnode:1".into())));
        let g2 = fc.features[1].geometry.as_ref().unwrap();
        assert_eq!(g2.r#type, proto::POLYGON);
        assert!(g2.lengths.is_empty()); // single ring omits lengths
        assert_eq!(g2.coords.len(), 6); // closing vertex dropped, delta encoded
        assert_eq!(g2.coords, vec![0, 0, 10000, 0, 0, 10000]);
        assert_eq!(fc.features[1].id_type, Some(proto::IdType::IntId(7)));
        assert_eq!(fc.features[1].values[2].value_type, Some(proto::ValueType::NegIntValue(3)));
        assert_eq!(fc.features[1].values[1].value_type, Some(proto::ValueType::JsonValue("null".into())));
    }
}
