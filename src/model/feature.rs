use super::Layer;
use geo_types::Geometry;
use serde_json::{Map, Value};

/// Property bag keyed by DO-272 attribute abbreviations (`idarpt`, `idrwy`, ...).
pub type Props = Map<String, Value>;

/// One AMDB feature. Geometry is in the *local metre frame* until output.
#[derive(Debug, Clone)]
pub struct AmdbFeature {
    pub layer: Layer,
    pub geom: Geometry<f64>,
    pub props: Props,
}

impl AmdbFeature {
    pub fn new(layer: Layer, geom: impl Into<Geometry<f64>>) -> Self {
        Self { layer, geom: geom.into(), props: Props::new() }
    }

    /// Builder-style property setter. `None` values are stored as JSON null so every
    /// feature of a layer carries the same key set.
    pub fn with(mut self, key: &str, v: impl Into<Value>) -> Self {
        self.props.insert(key.to_string(), v.into());
        self
    }

    pub fn set(&mut self, key: &str, v: impl Into<Value>) {
        self.props.insert(key.to_string(), v.into());
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.props.get(key).and_then(|v| v.as_str())
    }

    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.props.get(key).and_then(|v| v.as_f64())
    }
}

/// Helper to turn an `Option<T>` into a JSON value without dropping the key.
pub fn opt<T: Into<Value>>(v: Option<T>) -> Value {
    v.map(Into::into).unwrap_or(Value::Null)
}
