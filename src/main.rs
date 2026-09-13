//! amdbgen: builds a Navigraph-style Airport Mapping Database from free sources.

fn main() {
    let verbose = std::env::args().any(|a| a == "-v" || a == "--verbose") || std::env::var("AMDB_VERBOSE").is_ok();
    amdbgen::term::init(verbose);
    amdbgen::term::banner("amdbgen");
    if let Err(e) = amdbgen::cli::run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
