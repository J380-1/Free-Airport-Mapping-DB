//! `amdb-bridge` command line.
//!
//! `serve` is the whole integration: it redirects `amdb.api.navigraph.com` to this
//! machine through the hosts file, answers over HTTPS with a locally trusted
//! certificate, and removes the redirect again when it stops.

use super::{hosts, patcher, server, tls};
use super::store::Store;
use crate::build::BuildOptions;
use crate::cache::Cache;
use crate::output::{Formats, Projection};
use crate::pipeline::{Config, FaaMode, OsmMode};
use crate::sources::http::Http;
use anyhow::{anyhow, Result};
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "amdb-bridge", version, about = "Serve amdbgen airport data to aircraft in place of the Navigraph AMDB API")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args, Clone)]
struct DataArgs {
    /// Directory with generated airports (out/<ICAO>/...). Missing airports are built here on demand.
    #[arg(long, default_value = "out")]
    out: PathBuf,
    /// Optional X-Plane install to read apt.dat from instead of the Gateway.
    #[arg(long = "xplane-dir")]
    xplane_dir: Option<PathBuf>,
    /// Optional X-Plane earth_aptmeta.dat index.
    #[arg(long)]
    aptmeta: Option<PathBuf>,
    /// Optional on-disk download cache.
    #[arg(long)]
    cache: Option<PathBuf>,
}

#[derive(Args, Clone)]
struct ServeArgs {
    #[command(flatten)]
    data: DataArgs,
    /// HTTPS port for the redirected Navigraph host (the aircraft use 443).
    #[arg(long = "https-port", default_value_t = super::DEFAULT_HTTPS_PORT)]
    https_port: u16,
    /// Plain HTTP port for local tools; 0 disables it.
    #[arg(long = "http-port", default_value_t = super::DEFAULT_PORT)]
    http_port: u16,
    /// Do not touch the hosts file or the certificate store (HTTP/HTTPS only, no redirect).
    #[arg(long = "no-hosts")]
    no_hosts: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Redirect the Navigraph AMDB host to this machine and serve airports until stopped (Ctrl-C).
    Serve(ServeArgs),
    /// Build airports ahead of time so the first OANS load is instant.
    Prefetch {
        #[command(flatten)]
        data: DataArgs,
        icaos: Vec<String>,
        /// Also the airports of your latest SimBrief OFP (username or pilot id).
        #[arg(long)]
        simbrief: Option<String>,
    },
    /// Show redirect, certificate and aircraft status.
    Status,
    /// Remove the hosts-file redirect and the local certificate authority (cleanup after a crash).
    Cleanup,
    /// Add the server to the simulator's exe.xml so it starts with the sim.
    Autostart(ServeArgs),
    /// Alternative to the redirect: rewrite aircraft bundles to use http://127.0.0.1:PORT (backups kept).
    Patch {
        #[arg(long = "community")]
        community: Vec<PathBuf>,
        #[arg(long, default_value_t = super::DEFAULT_PORT)]
        port: u16,
        #[arg(long = "dry-run")]
        dry_run: bool,
    },
    /// Restore bundles changed by `patch`.
    Unpatch {
        #[arg(long = "community")]
        community: Vec<PathBuf>,
    },
}

fn config(d: &DataArgs) -> Config {
    Config {
        out: d.out.clone(),
        cache: Cache::new(d.cache.clone(), false, false),
        http: Http::new(300, 250),
        formats: Formats { geojson: true, pbf: false },
        projection: Projection::Wgs84,
        xplane_dir: d.xplane_dir.clone(),
        aptdat_file: None,
        use_gateway: true,
        osm: OsmMode::OsmApi,
        overpass_mirrors: crate::pipeline::default_mirrors(),
        aptmeta: d.aptmeta.clone(),
        ourairports: true,
        faa: FaaMode::Auto,
        overrides_dir: PathBuf::from("overrides"),
        radius_km: 3.0,
        build: BuildOptions::default(),
        write_ir: false,
        layers: crate::model::ALL_LAYERS.to_vec(),
        index_cache: Cache::for_index(false),
    }
}

/// Relaunch this command elevated (UAC prompt) and exit the current process.
fn relaunch_elevated() -> Result<()> {
    let exe = std::env::current_exe()?;
    let args: Vec<String> = std::env::args().skip(1).map(|a| format!("'{}'", a.replace('\'', "''"))).collect();
    let arg_list = if args.is_empty() { String::from("@()") } else { format!("@({})", args.join(",")) };
    let cmd = format!("Start-Process -FilePath '{}' -ArgumentList {} -Verb RunAs -WorkingDirectory '{}'", exe.display(), arg_list, std::env::current_dir()?.display());
    log::info!("administrator rights are needed for the hosts file; asking for elevation");
    let status = std::process::Command::new("powershell").args(["-NoProfile", "-Command", &cmd]).status()?;
    if !status.success() {
        return Err(anyhow!("elevation was refused"));
    }
    std::process::exit(0);
}

fn serve(a: ServeArgs) -> Result<()> {
    let domain = super::NAVIGRAPH_AMDB_DOMAIN;
    let mut https = None;
    if !a.no_hosts {
        if !hosts::writable() {
            relaunch_elevated()?;
        }
        let m = tls::ensure(domain)?;
        tls::trust(&m)?;
        hosts::install(domain)?;
        crate::term::success(&format!("Hosts file now sends {domain} here (removed automatically on exit)"));
        https = Some((a.https_port, m.cert_pem, m.key_pem));
        ctrlc::set_handler(move || {
            match hosts::remove() {
                Ok(_) => log::info!("hosts-file redirect removed"),
                Err(e) => log::error!("could not clean the hosts file: {e:#}"),
            }
            std::process::exit(0);
        })?;
    } else if a.https_port != super::DEFAULT_HTTPS_PORT || a.http_port == 0 {
        // Explicit HTTPS without the redirect (testing): still needs the certificate.
        let m = tls::ensure(domain)?;
        https = Some((a.https_port, m.cert_pem, m.key_pem));
    }
    let store = Store::new(config(&a.data))?;
    let result = server::serve(store, server::Listen { http_port: if a.http_port == 0 { None } else { Some(a.http_port) }, https });
    if !a.no_hosts {
        let _ = hosts::remove();
    }
    result
}

fn communities(extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut v = patcher::detect_community_dirs();
    for e in extra {
        if !v.contains(e) {
            v.push(e.clone());
        }
    }
    v
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Serve(a) => serve(a),
        Cmd::Prefetch { data, icaos, simbrief } => {
            let cfg = config(&data);
            let mut icaos = icaos;
            if let Some(user) = &simbrief {
                let ofp = crate::sources::simbrief::fetch(&cfg.http, user)?;
                crate::term::info(&format!("SimBrief {}: {}", ofp.flight.clone().unwrap_or_else(|| user.clone()), ofp.icaos().join(" → ")));
                icaos.extend(ofp.icaos());
            }
            if icaos.is_empty() {
                return Err(anyhow!("give ICAO codes or --simbrief to prefetch"));
            }
            let store = Store::new(cfg)?;
            for i in icaos {
                match store.airport(&i) {
                    Ok(a) => println!("{}: {} features ready", a.icao, a.layers.values().map(Vec::len).sum::<usize>()),
                    Err(e) => println!("{}: FAILED {e:#}", i.to_uppercase()),
                }
            }
            Ok(())
        }
        Cmd::Status => {
            let domain = super::NAVIGRAPH_AMDB_DOMAIN;
            println!("hosts redirect for {domain}: {}", if hosts::is_installed(domain) { "ACTIVE" } else { "not installed" });
            println!("local CA trusted by Windows: {}", if tls::is_trusted() { "yes" } else { "no" });
            println!("certificate folder: {}", tls::data_dir().display());
            for d in communities(&[]) {
                println!("Community: {}", d.display());
                for c in patcher::scan(&d) {
                    println!("  {}  references the Navigraph AMDB host ({} refs) -> covered by the redirect", c.package, c.literal_hits + c.template_hits);
                }
                for f in patcher::load_record(&d).files {
                    println!("  PATCHED {}", f.path.display());
                }
            }
            Ok(())
        }
        Cmd::Cleanup => {
            if !hosts::writable() {
                relaunch_elevated()?;
            }
            println!("hosts redirect removed: {}", hosts::remove()?);
            println!("local CA removed from trust store: {}", tls::untrust()?);
            Ok(())
        }
        Cmd::Autostart(a) => {
            let exe = std::env::current_exe()?;
            let out = std::fs::canonicalize(&a.data.out).unwrap_or(a.data.out.clone());
            let args = format!("serve --out \"{}\"", out.display());
            let written = patcher::install_autostart(&exe, &args)?;
            if written.is_empty() {
                println!("no exe.xml found or entry already present");
            }
            for p in written {
                println!("added AMDB Bridge to {} (it will ask for administrator rights when the sim starts)", p.display());
            }
            Ok(())
        }
        Cmd::Patch { community, port, dry_run } => {
            let mut total = 0;
            for d in communities(&community) {
                total += patcher::patch(&d, port, dry_run)?.len();
            }
            println!("{}{} file(s) patched", if dry_run { "[dry-run] " } else { "" }, total);
            Ok(())
        }
        Cmd::Unpatch { community } => {
            let mut total = 0;
            for d in communities(&community) {
                total += patcher::unpatch(&d)?;
            }
            println!("{total} file(s) restored");
            Ok(())
        }
    }
}
