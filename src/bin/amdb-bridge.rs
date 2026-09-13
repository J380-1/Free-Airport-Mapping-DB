//! amdb-bridge: serves amdbgen data to aircraft through the Navigraph AMDB API
//! surface, and patches aircraft packages to use it.

fn main() {
    let verbose = std::env::args().any(|a| a == "-v" || a == "--verbose") || std::env::var("AMDB_VERBOSE").is_ok();
    amdbgen::term::init(verbose);
    amdbgen::term::banner("amdb-bridge");
    if let Err(e) = amdbgen::bridge::cli::run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
