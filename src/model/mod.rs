//! AMDB feature model: the 45 DO-272 / AMXM feature types, their geometry kinds,
//! and a generic feature container keyed by DO-272 attribute abbreviations.

pub mod codes;
pub mod feature;
pub mod layer;

pub use feature::{AmdbFeature, Props};
pub use layer::{GeomKind, Layer, ALL_LAYERS};
