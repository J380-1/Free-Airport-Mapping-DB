//! Aircraft bridge: a local server that speaks the Navigraph AMDB API surface the
//! aircraft already use (FlyByWire A380X/A32NX OANC, and any add-on built on the
//! same API), fed by amdbgen data; plus a patcher that points installed aircraft
//! packages at it.

pub mod cli;
pub mod compat;
pub mod hosts;
pub mod patcher;
pub mod server;
pub mod store;
pub mod tls;

pub const DEFAULT_PORT: u16 = 8770;
pub const NAVIGRAPH_AMDB_HOST: &str = "https://amdb.api.navigraph.com";
pub const NAVIGRAPH_AMDB_DOMAIN: &str = "amdb.api.navigraph.com";
pub const DEFAULT_HTTPS_PORT: u16 = 443;
