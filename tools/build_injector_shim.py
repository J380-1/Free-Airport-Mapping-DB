"""Package and install the AMDB token shim for encrypted iniBuilds aircraft.

The shim is a minimal toolbar panel (no map of its own): while it is open it answers
the OANS gauge's Navigraph token requests over the comm bus with the bridge
placeholder, so the aircraft's OWN airport map draws from amdb-bridge. It exists for
aircraft whose EFB files are sealed and cannot be patched, e.g. the Marketplace
iniBuilds A380.

Writes layout.json, stamps the package size into the manifest, compiles the toolbar
registration (.spb) when the MSFS SDK is available, and copies the result into every
Community folder found (MSFS 2020 and 2024). The Build/ directory holds the SDK
sources for the .spb and is never installed; the compiled
InGamePanels/amdb-injector-shim.spb is committed so no SDK is needed to install.

    python tools/build_injector_shim.py --dry-run     # show what would be installed
    python tools/build_injector_shim.py               # build and install
    python tools/build_injector_shim.py --to DIR      # build into a folder of your own
"""

import argparse
import json
import os
import shutil
import subprocess
import sys

SRC = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "packages", "msfs-injector-shim")
FOLDER = "amdb-injector-shim"
SPB = os.path.join("InGamePanels", "amdb-injector-shim.spb")
SKIP_DIRS = {"Build"}


def filetime(path):
    """Windows FILETIME: 100-nanosecond ticks since 1601, which is what layout.json wants."""
    return int((os.path.getmtime(path) + 11644473600) * 10_000_000)


def package_files(root):
    """Every file that belongs in layout.json: everything but layout.json and Build/."""
    out = []
    for dirpath, dirnames, names in os.walk(root):
        dirnames[:] = sorted(d for d in dirnames if d not in SKIP_DIRS)
        for name in sorted(names):
            full = os.path.join(dirpath, name)
            rel = os.path.relpath(full, root).replace("\\", "/")
            if rel == "layout.json":
                continue
            out.append((rel, full))
    return sorted(out)


def find_fspackagetool():
    """fspackagetool.exe from the MSFS SDK, if one is installed."""
    candidates = [
        os.path.join(os.environ.get("MSFS_SDK", ""), "Tools", "bin", "fspackagetool.exe"),
        r"C:\MSFS SDK\Tools\bin\fspackagetool.exe",
    ]
    path_dirs = os.environ.get("PATH", "").split(os.pathsep)
    for directory in path_dirs:
        candidates.append(os.path.join(directory, "fspackagetool.exe"))
    for candidate in candidates:
        if candidate and os.path.isfile(candidate):
            return candidate
    return None


def build_spb(root):
    """Compile the toolbar registration. Returns True when the .spb is in place."""
    dest = os.path.join(root, SPB)
    if os.path.isfile(dest):
        return True
    project = os.path.join(root, "Build", "amdb-injector-shim.xml")
    tool = find_fspackagetool()
    if not os.path.isfile(project):
        print("Panel sources missing (Build/amdb-injector-shim.xml); cannot build " + SPB)
        return False
    if tool is None:
        print("fspackagetool.exe not found (MSFS SDK); cannot build " + SPB)
        print("Install the MSFS SDK or set MSFS_SDK, then rerun.")
        return False
    print("Compiling %s ..." % SPB)
    proc = subprocess.run([tool, project], cwd=root, capture_output=True, text=True)
    print(proc.stdout[-2000:] if proc.stdout else "")
    if proc.returncode != 0:
        print(proc.stderr[-2000:] if proc.stderr else "")
        return os.path.isfile(dest)
    # The tool emits under Build/Packages; move the .spb to the package root.
    for dirpath, _, names in os.walk(os.path.join(root, "Build", "Packages")):
        for name in names:
            if name.lower().endswith(".spb"):
                shutil.copy(os.path.join(dirpath, name), dest)
                print("Wrote " + dest)
                return True
    return os.path.isfile(dest)


def build(root):
    """Write layout.json and update the manifest's total size, in place."""
    if not build_spb(root):
        raise SystemExit(1)
    entries = []
    total = 0
    for rel, full in package_files(root):
        size = os.path.getsize(full)
        total += size
        # MSFS records these paths lower-cased.
        entries.append({"path": rel.lower(), "size": size, "date": filetime(full)})
    with open(os.path.join(root, "layout.json"), "w", encoding="utf-8") as fh:
        json.dump({"content": entries}, fh, indent=2)
        fh.write("\n")

    mpath = os.path.join(root, "manifest.json")
    with open(mpath, encoding="utf-8") as fh:
        manifest = json.load(fh)
    manifest["total_package_size"] = str(total).zfill(20)
    with open(mpath, "w", encoding="utf-8") as fh:
        json.dump(manifest, fh, indent=2)
        fh.write("\n")
    return entries, total


def community_dirs():
    """Community folders for MSFS 2020 and 2024 (Store and Steam)."""
    out = []
    local = os.environ.get("LOCALAPPDATA", "")
    roaming = os.environ.get("APPDATA", "")
    for cfg in [
        os.path.join(local, "Packages/Microsoft.FlightSimulator_8wekyb3d8bbwe/LocalCache/UserCfg.opt"),
        os.path.join(local, "Packages/Microsoft.Limitless_8wekyb3d8bbwe/LocalCache/UserCfg.opt"),
        os.path.join(roaming, "Microsoft Flight Simulator/UserCfg.opt"),
        os.path.join(roaming, "Microsoft Flight Simulator 2024/UserCfg.opt"),
    ]:
        try:
            with open(cfg, encoding="utf-8", errors="ignore") as fh:
                for line in fh:
                    line = line.strip()
                    if line.startswith("InstalledPackagesPath"):
                        d = os.path.normpath(os.path.join(line.split(None, 1)[1].strip().strip('"'), "Community"))
                        if os.path.isdir(d) and d not in out:
                            out.append(d)
        except (OSError, IndexError):
            pass
    for d in [
        os.path.join(local, "Packages/Microsoft.FlightSimulator_8wekyb3d8bbwe/LocalCache/Packages/Community"),
        os.path.join(local, "Packages/Microsoft.Limitless_8wekyb3d8bbwe/LocalCache/Packages/Community"),
    ]:
        d = os.path.normpath(d)
        if os.path.isdir(d) and d not in out:
            out.append(d)
    return out


def uninstall(targets, dry_run):
    """Take the shim out again."""
    gone = 0
    for community in targets:
        dest = os.path.join(community, FOLDER)
        if os.path.isdir(dest):
            print("%s" % dest)
            if dry_run:
                print("   [dry-run] would remove")
            else:
                shutil.rmtree(dest)
                print("   removed")
            gone += 1
    if not gone:
        print("The shim is not installed in any Community folder found.")
    elif not dry_run:
        print("\nDone. Restart the sim for the change to take effect.")
    return 0


def main():
    ap = argparse.ArgumentParser(description="Build and install the AMDB token shim.")
    ap.add_argument("--to", action="append", metavar="COMMUNITY", help="install into this folder (repeatable); default: every one detected")
    ap.add_argument("--dry-run", action="store_true", help="report what would happen, copy nothing")
    ap.add_argument("--uninstall", action="store_true", help="remove the shim again")
    args = ap.parse_args()

    if args.uninstall:
        return uninstall(args.to or community_dirs(), args.dry_run)

    if not os.path.isdir(SRC):
        print("Package source not found:", SRC)
        return 1

    entries, total = build(SRC)
    print("Built %s: %d files, %.1f kB" % (FOLDER, len(entries), total / 1000))
    for e in entries:
        print("   %-72s %7d" % (e["path"], e["size"]))

    targets = args.to or community_dirs()
    if not targets:
        print("\nNo Community folder found. Pass one with --to.")
        return 1

    print()
    for community in targets:
        dest = os.path.join(community, FOLDER)
        sim = "MSFS 2024" if ("Limitless" in community or "2024" in community) else "MSFS 2020"
        print("%s  %s" % (sim, dest))
        if args.dry_run:
            print("   [dry-run] would copy %d files" % len(entries))
            continue
        if os.path.isdir(dest):
            shutil.rmtree(dest)
        shutil.copytree(SRC, dest, ignore=shutil.ignore_patterns("Build"))
        print("   installed")

    if not args.dry_run:
        print("\nStart `amdb-bridge serve`, open the AMDB Token Shim toolbar panel,")
        print("keep it open, and load the A380. Its own airport map should then draw.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
