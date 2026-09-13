//! Overpass API client (free, no key). Queries everything airport-relevant inside a
//! bounding box and parses the JSON into a `Store`.

use super::elements::{Member, Relation, Store, Tags, Way};
use crate::cache::Cache;
use crate::sources::http::Http;
use anyhow::{anyhow, Context, Result};
use geo_types::Coord;
use serde_json::Value;

pub const DEFAULT_MIRRORS: &[&str] = &[
    "https://overpass-api.de/api/interpreter",
    "https://overpass.kumi.systems/api/interpreter",
    "https://overpass.private.coffee/api/interpreter",
];

/// Build the Overpass QL query for a bbox (south, west, north, east).
pub fn query(bbox: (f64, f64, f64, f64)) -> String {
    let (s, w, n, e) = bbox;
    let bb = format!("({s:.6},{w:.6},{n:.6},{e:.6})");
    let mut q = String::from("[out:json][timeout:180];(");
    for sel in [
        // Airport features (nodes are needed for stands/holds/lights, ways/relations for areas).
        "nwr[\"aeroway\"][\"aeroway\"!=\"navigationaid\"]",
        "node[\"aeroway\"=\"navigationaid\"][\"navigationaid\"~\"papi|vasi|als|reil\"]",
        // Buildings and structures.
        "wr[\"building\"]",
        "nwr[\"man_made\"~\"^(tower|mast|chimney|antenna|storage_tank|silo|communications_tower|water_tower|lighthouse)$\"]",
        "way[\"barrier\"~\"^(fence|wall)$\"]",
        "way[\"power\"~\"^(line|minor_line)$\"]",
        // Airside service roads only (no parking aisles, no public road network).
        "way[\"highway\"~\"^(service|unclassified|living_street|track)$\"][\"service\"!~\"parking_aisle|driveway|drive-through|emergency_access\"]",
        // Areas.
        "wr[\"natural\"=\"water\"]",
        "wr[\"water\"]",
        "wr[\"landuse\"~\"^(construction|reservoir|basin)$\"]",
        "wr[\"construction\"]",
        "wr[\"deicing\"]",
    ] {
        q.push_str(sel);
        q.push_str(&bb);
        q.push(';');
    }
    q.push_str(");out body;>;out skel qt;");
    q
}

/// Parse an Overpass JSON response into a `Store`.
pub fn parse_json(text: &str) -> Result<Store> {
    let v: Value = serde_json::from_str(text).context("overpass json")?;
    if let Some(rem) = v.get("remark").and_then(Value::as_str) {
        if rem.contains("error") || rem.contains("timed out") {
            return Err(anyhow!("overpass remark: {rem}"));
        }
    }
    let mut st = Store::default();
    let Some(els) = v.get("elements").and_then(Value::as_array) else { return Err(anyhow!("overpass: no elements")) };
    for e in els {
        let id = e.get("id").and_then(Value::as_i64).unwrap_or(0);
        let tags: Tags = e
            .get("tags")
            .and_then(Value::as_object)
            .map(|m| m.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
            .unwrap_or_default();
        match e.get("type").and_then(Value::as_str) {
            Some("node") => {
                if let (Some(lat), Some(lon)) = (e.get("lat").and_then(Value::as_f64), e.get("lon").and_then(Value::as_f64)) {
                    st.nodes.insert(id, Coord { x: lon, y: lat });
                    if !tags.is_empty() {
                        st.node_tags.insert(id, tags);
                    }
                }
            }
            Some("way") => {
                let nodes = e.get("nodes").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_i64).collect()).unwrap_or_default();
                st.ways.insert(id, Way { id, nodes, tags });
            }
            Some("relation") => {
                let members = e
                    .get("members")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|m| {
                                let kind = match m.get("type").and_then(Value::as_str)? {
                                    "node" => 'n',
                                    "way" => 'w',
                                    _ => 'r',
                                };
                                Some(Member { kind, id: m.get("ref").and_then(Value::as_i64)?, role: m.get("role").and_then(Value::as_str).unwrap_or("").to_string() })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                st.relations.insert(id, Relation { id, members, tags });
            }
            _ => {}
        }
    }
    Ok(st)
}

/// Fetch (cached) Overpass data for an airport bbox, trying each mirror in turn.
pub fn fetch(http: &Http, cache: &Cache, mirrors: &[String], icao: &str, bbox: (f64, f64, f64, f64)) -> Result<Store> {
    let key = format!("osm/overpass/{}.json", icao.to_uppercase());
    let q = query(bbox);
    let text = cache.get_or_fetch_text(&key, || {
        let mut last = None;
        // A throttled (429) or busy mirror is skipped at once; the next mirror gets the query.
        for round in 0..3 {
            for m in mirrors {
                match http.post_form_text_once(m, &[("data", q.as_str())]) {
                    Ok(t) => match parse_json(&t) {
                        Ok(_) => return Ok(t),
                        Err(e) => {
                            log::warn!("overpass {m}: bad response: {e:#}");
                            last = Some(e);
                        }
                    },
                    Err(e) => {
                        log::warn!("overpass {m}: {e:#}");
                        last = Some(e);
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(5 * (round + 1)));
        }
        Err(last.unwrap_or_else(|| anyhow!("no overpass mirror answered")))
    })?;
    parse_json(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_overpass_json() {
        let j = r#"{"version":0.6,"elements":[
          {"type":"node","id":1,"lat":28.5,"lon":77.1},
          {"type":"node","id":2,"lat":28.6,"lon":77.2,"tags":{"aeroway":"parking_position","ref":"5"}},
          {"type":"way","id":10,"nodes":[1,2],"tags":{"aeroway":"taxiway","ref":"A"}},
          {"type":"relation","id":20,"members":[{"type":"way","ref":10,"role":"outer"}],"tags":{"type":"multipolygon","aeroway":"aerodrome"}}
        ]}"#;
        let st = parse_json(j).unwrap();
        assert_eq!(st.nodes.len(), 2);
        assert_eq!(st.node_tags[&2]["ref"], "5");
        assert_eq!(st.ways[&10].nodes, vec![1, 2]);
        assert_eq!(st.relations[&20].members[0].kind, 'w');
        assert!(parse_json("<html>busy</html>").is_err());
        assert!(query((28.0, 77.0, 28.1, 77.1)).contains("(28.000000,77.000000,28.100000,77.100000);"));
    }
}
