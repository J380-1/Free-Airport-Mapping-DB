//! Planar geometry helpers. All build-time geometry lives in a local
//! azimuthal-equidistant metre frame centred on the aerodrome reference point.

pub mod bezier;
pub mod frame;
pub mod ops;

pub use frame::LocalFrame;
