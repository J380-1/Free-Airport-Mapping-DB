//! Command line interface.

use crate::build::BuildOptions;
use crate::cache::Cache;
use crate::output::{Formats, Projection};
use crate::pipeline::{self, Config, FaaMode, OsmMode};
use crate::sources::http::Http;
use anyhow::{anyhow, Result};
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "amdbgen", version, about = "Build a Navigraph-style Airport Mapping Database (DO-272 layers) from free sources")]
struct Cli {
    /// Show debug output.
    #[arg(short, long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Build AMDB layers for airports (GeoJSON + PBF per layer).
    Build(BuildArgs),
    /// Validate a generated airport directory (out/<ICAO>).
    Validate { dir: PathBuf },
    /// List ICAOs the index knows for a selection (country/region/prefix).
    List(SelectArgs),
    /// Pre-download source data into the cache without building.
    Fetch(BuildArgs),
}

#[derive(Args, Clone)]
pub struct SelectArgs {
    /// ICAO codes to build (e.g. EDDF VIDP KJFK).
    icaos: Vec<String>,
    /// All airports in an ISO country (needs OurAirports or aptmeta index), e.g. IN, DE.
    #[arg(long)]
    country: Option<String>,
    /// All airports whose aptmeta region starts with this (e.g. VI, K2).
    #[arg(long)]
    region: Option<String>,
    /// All airports whose ICAO starts with this prefix (e.g. ED, VA).
    #[arg(long = "icao-prefix")]
    prefix: Option<String>,
    /// Every airport in the index (worldwide).
    #[arg(long)]
    all: bool,
    /// Add the airports of your latest SimBrief OFP (username or pilot id).
    #[arg(long)]
    simbrief: Option<String>,
    /// X-Plane earth_aptmeta.dat (airport index with ARP/elevation/transition levels).
    #[arg(long)]
    aptmeta: Option<PathBuf>,
    /// Do not download the OurAirports index.
    #[arg(long = "no-ourairports")]
    no_ourairports: bool,
    /// Optional on-disk cache for downloads (off by default: every run refetches).
    #[arg(long)]
    cache: Option<PathBuf>,
    /// Never use the network (only meaningful with --cache).
    #[arg(long, requires = "cache")]
    offline: bool,
    /// Ignore cached downloads and refetch (only meaningful with --cache).
    #[arg(long, requires = "cache")]
    refresh: bool,
}

#[derive(Args, Clone)]
pub struct BuildArgs {
    #[command(flatten)]
    select: SelectArgs,
    /// Output directory.
    #[arg(long, default_value = "out")]
    out: PathBuf,
    /// Output formats: geojson, pbf, or geojson,pbf.
    #[arg(long, default_value = "geojson,pbf")]
    format: String,
    /// Coordinate output: wgs84 (EPSG:4326) or metres (azimuthal equidistant from ARP).
    #[arg(long, default_value = "wgs84")]
    projection: String,
    /// X-Plane installation root (Custom Scenery / Global Airports apt.dat are used).
    #[arg(long = "xplane-dir")]
    xplane_dir: Option<PathBuf>,
    /// A specific apt.dat file (any size) to read airports from.
    #[arg(long = "aptdat")]
    aptdat_file: Option<PathBuf>,
    /// Do not query the X-Plane Scenery Gateway.
    #[arg(long = "no-gateway")]
    no_gateway: bool,
    /// OSM source: osmapi (default: direct OpenStreetMap API, Overpass fallback), overpass, or off.
    #[arg(long, default_value = "osmapi")]
    osm: String,
    /// Overpass mirror(s), comma separated.
    #[arg(long)]
    overpass: Option<String>,
    /// FAA NASR enrichment: auto (US airports only), off, or a path to the CSV zip/dir.
    #[arg(long, default_value = "auto")]
    faa: String,
    /// Directory with overrides/<ICAO>/<layer>.geojson.
    #[arg(long, default_value = "overrides")]
    overrides: PathBuf,
    /// OSM query radius around the ARP (km) when the airport extent is unknown.
    #[arg(long, default_value_t = 3.0)]
    radius_km: f64,
    /// Parallel jobs.
    #[arg(long, short = 'j')]
    jobs: Option<usize>,
    /// Do not generate Annex 14 runway markings.
    #[arg(long = "no-markings")]
    no_markings: bool,
    /// Do not derive taxiway shoulders.
    #[arg(long = "no-shoulders")]
    no_shoulders: bool,
    /// Also write the merged source model (_source.json) for debugging.
    #[arg(long = "write-source")]
    write_source: bool,
    /// HTTP timeout in seconds.
    #[arg(long, default_value_t = 300)]
    timeout: u64,
    /// Which layers to write: full (all 45) or map (the 34 a moving map draws).
    #[arg(long, default_value = "full")]
    profile: String,
    /// Explicit comma-separated layer list (overrides --profile), e.g. runwayelement,taxiwayelement.
    #[arg(long)]
    layers: Option<String>,
}

pub fn config(a: &BuildArgs) -> Result<Config> {
    let formats = Formats { geojson: a.format.contains("geojson") || a.format.contains("json"), pbf: a.format.contains("pbf") };
    if !formats.geojson && !formats.pbf {
        return Err(anyhow!("--format must include geojson and/or pbf"));
    }
    let projection = match a.projection.to_ascii_lowercase().as_str() {
        "wgs84" | "4326" | "epsg:4326" | "latlon" => Projection::Wgs84,
        "metres" | "meters" | "local" | "aeqd" => Projection::LocalMetres,
        other => return Err(anyhow!("unknown projection {other}")),
    };
    let osm = match a.osm.to_ascii_lowercase().as_str() {
        "osmapi" | "api" | "osm" => OsmMode::OsmApi,
        "overpass" => OsmMode::Overpass,
        "off" | "none" => OsmMode::Off,
        other => return Err(anyhow!("unknown --osm mode {other}")),
    };
    let faa = match a.faa.to_ascii_lowercase().as_str() {
        "auto" => FaaMode::Auto,
        "off" | "none" => FaaMode::Off,
        _ => FaaMode::File(PathBuf::from(&a.faa)),
    };
    let mirrors = a.overpass.as_ref().map(|s| s.split(',').map(|m| m.trim().to_string()).collect()).unwrap_or_else(pipeline::default_mirrors);
    let build = BuildOptions { runway_markings: !a.no_markings, derive_shoulders: !a.no_shoulders, ..Default::default() };
    let layers: Vec<crate::model::Layer> = if let Some(list) = &a.layers {
        let mut v = Vec::new();
        for name in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            v.push(crate::model::Layer::from_name(name).ok_or_else(|| anyhow!("unknown layer {name}"))?);
        }
        v
    } else {
        match a.profile.to_ascii_lowercase().as_str() {
            "full" | "all" => crate::model::ALL_LAYERS.to_vec(),
            "map" | "oans" => crate::model::layer::MAP_PROFILE.to_vec(),
            other => return Err(anyhow!("unknown profile {other} (full|map)")),
        }
    };
    Ok(Config {
        out: a.out.clone(),
        cache: Cache::new(a.select.cache.clone(), a.select.offline, a.select.refresh),
        http: Http::new(a.timeout, 250),
        formats,
        projection,
        xplane_dir: a.xplane_dir.clone(),
        aptdat_file: a.aptdat_file.clone(),
        use_gateway: !a.no_gateway,
        osm,
        overpass_mirrors: mirrors,
        aptmeta: a.select.aptmeta.clone(),
        ourairports: !a.select.no_ourairports,
        faa,
        overrides_dir: a.overrides.clone(),
        radius_km: a.radius_km,
        build,
        write_ir: a.write_source,
        layers,
        index_cache: Cache::for_index(a.select.offline),
    })
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Build(a) | Cmd::Fetch(a) => {
            if let Some(j) = a.jobs {
                rayon::ThreadPoolBuilder::new().num_threads(j).build_global().ok();
            }
            let cfg = config(&a)?;
            let s = &a.select;
            let mut icaos = pipeline::select_icaos(&cfg, &s.icaos, s.country.as_deref(), s.region.as_deref(), s.prefix.as_deref(), s.all)?;
            if let Some(user) = &s.simbrief {
                let ofp = crate::sources::simbrief::fetch(&cfg.http, user)?;
                crate::term::info(&format!("SimBrief {}: {}", ofp.flight.clone().unwrap_or_else(|| user.clone()), ofp.icaos().join(" → ")));
                for i in ofp.icaos() {
                    if !icaos.contains(&i) {
                        icaos.push(i);
                    }
                }
            }
            if icaos.is_empty() {
                return Err(anyhow!("no airports selected (give ICAO codes, --simbrief, or --country/--region/--icao-prefix/--all)"));
            }
            let summary = pipeline::run(&cfg, &icaos)?;
            if !summary.failed.is_empty() {
                for (icao, e) in &summary.failed {
                    crate::term::error(&format!("[{icao}] {e}"));
                }
                return Err(anyhow!("{} airport(s) failed", summary.failed.len()));
            }
            Ok(())
        }
        Cmd::Validate { dir } => {
            let rep = crate::validate::validate_dir(&dir)?;
            for e in &rep.errors {
                println!("ERROR {e}");
            }
            for w in &rep.warnings {
                println!("WARN  {w}");
            }
            println!("{}: {} error(s), {} warning(s)", dir.display(), rep.errors.len(), rep.warnings.len());
            if rep.errors.is_empty() { Ok(()) } else { Err(anyhow!("validation failed")) }
        }
        Cmd::List(s) => {
            let a = BuildArgs {
                select: s.clone(),
                out: PathBuf::from("out"),
                format: "geojson".into(),
                projection: "wgs84".into(),
                xplane_dir: None,
                aptdat_file: None,
                no_gateway: true,
                osm: "off".into(),
                overpass: None,
                faa: "off".into(),
                overrides: PathBuf::from("overrides"),
                radius_km: 5.0,
                jobs: None,
                no_markings: false,
                no_shoulders: false,
                write_source: false,
                timeout: 120,
                profile: "full".into(),
                layers: None,
            };
            let cfg = config(&a)?;
            let icaos = pipeline::select_icaos(&cfg, &s.icaos, s.country.as_deref(), s.region.as_deref(), s.prefix.as_deref(), s.all)?;
            for i in &icaos {
                println!("{i}");
            }
            eprintln!("{} airport(s)", icaos.len());
            Ok(())
        }
    }
}
