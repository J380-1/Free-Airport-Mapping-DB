//! SimBrief: read the latest OFP for a username or pilot id (free, no key) and
//! return the airports it uses, so a flight's origin, destination and alternates can
//! be built or prefetched in one go.

use crate::sources::http::Http;
use anyhow::{anyhow, Context, Result};
use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct Ofp {
    pub origin: Option<String>,
    pub destination: Option<String>,
    pub alternates: Vec<String>,
    pub flight: Option<String>,
    pub aircraft: Option<String>,
}

impl Ofp {
    /// All distinct ICAOs in order origin, destination, alternates.
    pub fn icaos(&self) -> Vec<String> {
        let mut v: Vec<String> = self.origin.iter().chain(self.destination.iter()).chain(self.alternates.iter()).map(|s| s.to_uppercase()).collect();
        v.dedup();
        let mut seen = std::collections::HashSet::new();
        v.retain(|s| seen.insert(s.clone()));
        v
    }
}

fn icao_of(v: &Value) -> Option<String> {
    v.get("icao_code").and_then(Value::as_str).filter(|s| s.len() == 4).map(str::to_string)
}

/// Parse a SimBrief JSON OFP.
pub fn parse(text: &str) -> Result<Ofp> {
    let v: Value = serde_json::from_str(text).context("simbrief json")?;
    if let Some(err) = v.get("fetch").and_then(|f| f.get("status")).and_then(Value::as_str).filter(|s| *s != "Success") {
        return Err(anyhow!("simbrief: {err}"));
    }
    let mut ofp = Ofp::default();
    ofp.origin = v.get("origin").and_then(icao_of);
    ofp.destination = v.get("destination").and_then(icao_of);
    match v.get("alternate") {
        Some(Value::Array(a)) => ofp.alternates = a.iter().filter_map(icao_of).collect(),
        Some(o @ Value::Object(_)) => ofp.alternates = icao_of(o).into_iter().collect(),
        _ => {}
    }
    ofp.flight = v.get("general").and_then(|g| g.get("flight_number")).and_then(Value::as_str).map(str::to_string).or_else(|| v.get("atc").and_then(|a| a.get("callsign")).and_then(Value::as_str).map(str::to_string));
    ofp.aircraft = v.get("aircraft").and_then(|a| a.get("icaocode")).and_then(Value::as_str).map(str::to_string);
    if ofp.origin.is_none() && ofp.destination.is_none() {
        return Err(anyhow!("simbrief: no airports in the OFP"));
    }
    Ok(ofp)
}

/// Fetch the latest OFP for a SimBrief username or numeric pilot id.
pub fn fetch(http: &Http, user: &str) -> Result<Ofp> {
    let param = if user.chars().all(|c| c.is_ascii_digit()) { "userid" } else { "username" };
    let url = format!("https://www.simbrief.com/api/xml.fetcher.php?{param}={user}&json=1");
    let text = http.get_text(&url)?;
    parse(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ofp_airports() {
        let j = r#"{"fetch":{"status":"Success"},"general":{"flight_number":"123"},"aircraft":{"icaocode":"A388"},
          "origin":{"icao_code":"EDDF"},"destination":{"icao_code":"KJFK"},
          "alternate":[{"icao_code":"KEWR"},{"icao_code":"KBOS"}]}"#;
        let o = parse(j).unwrap();
        assert_eq!(o.icaos(), vec!["EDDF", "KJFK", "KEWR", "KBOS"]);
        assert_eq!(o.aircraft.as_deref(), Some("A388"));
        let single = r#"{"origin":{"icao_code":"VIDP"},"destination":{"icao_code":"VABB"},"alternate":{"icao_code":"VAAH"}}"#;
        assert_eq!(parse(single).unwrap().icaos(), vec!["VIDP", "VABB", "VAAH"]);
        assert!(parse(r#"{"fetch":{"status":"Error: Unknown UserID"}}"#).is_err());
    }
}
