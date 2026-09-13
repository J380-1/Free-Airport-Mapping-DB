#!/usr/bin/env python3
"""Render an animated terminal recording (SVG, loops, no dependencies) of amdbgen
output for the README. Colours and symbols match the real terminal output.

Usage: python tools/make_demo_svg.py [docs/cli.svg]
"""
import html
import sys

# Palette (GitHub-dark-ish terminal).
BG, FG, DIM = "#0c0c0c", "#cccccc", "#767676"
CYAN, GREEN, BLUE, MAGENTA, YELLOW = "#569cd6", "#6a9955", "#569cd6", "#b180d7", "#d7ba7d"

# Each line: list of (text, colour, bold) segments. None = blank line.
def scope(s):
    return [(f"[{s}] ", DIM, False), ("» ", DIM, False)]

def info(msg, s=None):
    return (scope(s) if s else []) + [("i ", CYAN, False), ("info", CYAN, "u"), ("     ", FG, False), (msg, FG, False)]

def start(msg):
    return [("▶ ", GREEN, False), ("start", GREEN, "u"), ("    ", FG, False), (msg, FG, False)]

def step(msg, s):
    return scope(s) + [("· ", BLUE, False), ("step", BLUE, "u"), ("     ", FG, False), (msg, FG, False)]

def layer(i, n, name, count, s):
    c = (f"{count} features", GREEN, False) if count else ("empty", DIM, False)
    return scope(s) + [("· ", MAGENTA, False), ("layer", MAGENTA, "u"), ("    ", FG, False), (f"{i:>2}/{n} ", DIM, False), (f"{name:<34}", FG, False), c]

def success(msg, s=None):
    return (scope(s) if s else []) + [("✓ ", GREEN, False), ("success", GREEN, "u"), ("  ", FG, False), (msg, FG, False)]

def file_(path, detail, s):
    return scope(s) + [("  ", FG, False), ("file", BLUE, "u"), ("     ", FG, False), (path + " ", DIM, False), ("— ", DIM, False), (detail, BLUE, False)]

def prompt(cmd):
    return [("Free-Airport-Mapping-DB ", CYAN, True), ("on ", FG, False), (" main ", MAGENTA, True), ("[!] ", "#e06c75", False), ("is ", FG, False), ("📦 v0.1.0 ", YELLOW, True), ("via ", FG, False), ("🦀 v1.97.1", "#e06c75", True)]

def prompt2(cmd):
    return [("❯ ", GREEN, False), (cmd, YELLOW, False)]

E = "EDDF"
LINES = [
    prompt2("amdbgen build EDDF"),
    info("Welcome to amdbgen, v0.1.0"),
    info("Index: 44,692 airports from OurAirports"),
    info("Index: 41,222 airports on the X-Plane Gateway"),
    None,
    start("Fetching sources for 1 airport"),
    step("GET https://gateway.x-plane.com/apiv1/scenery/103368", E),
    step("Scenery pack 103368 received: 1.44 MB zip, apt.dat 1.09 MB in 1.8 s", E),
    info("X-Plane apt.dat: 4 runways, 77 pavements, 227 stands, 670 taxi routes", E),
    step("GET https://api.openstreetmap.org/api/0.6/map  bbox 8.4996,49.9986 to 8.6182,50.0570", E),
    step("OSM tile over 50k nodes, splitting into 4 (depth 1)", E),
    step("OSM tile 1: 9.92 MB (38,120 new nodes)", E),
    step("OSM tile 4: 8.31 MB (31,904 new nodes)", E),
    info("OpenStreetMap: 148,806 nodes, 23,305 ways in 9.6 s", E),
    None,
    start("Building 1 airport"),
    step("Runways, thresholds, intersections, shoulders in 41 ms", E),
    step("Runway markings (Annex 14) in 12 ms", E),
    step("Taxiway and apron pavement in 1.2 s", E),
    step("Guidance lines, holds, exits in 380 ms", E),
    step("Routing network (ASRN) in 95 ms", E),
    step("Buildings, roads, water, frequencies, helipads in 610 ms", E),
    layer(2, 45, "aerodromereferencepoint", 1, E),
    layer(5, 45, "apronelement", 40, E),
    layer(8, 45, "asrnedge", 1087, E),
    layer(18, 45, "hotspot", 0, E),
    layer(26, 45, "runwayelement", 4, E),
    layer(36, 45, "taxiwayelement", 65, E),
    layer(37, 45, "taxiwayguidanceline", 1337, E),
    layer(44, 45, "verticalpolygonalstructure", 560, E),
    info("Derived 45 layers, 10,999 features in 3.1 s", E),
    info("Validated: no issues", E),
    success("Built EDDF in 4.0 s (10,999 features)", E),
    file_("out/EDDF/*.geojson", "45 layers, 5.42 MB", E),
    file_("out/EDDF/*.pbf", "45 layers, 1.70 MB", E),
    file_("out/EDDF/manifest.json", "sources, counts, warnings", E),
    None,
    success("Built 1 airport in 16.6 s"),
]

W, PAD, LH, TOP = 760, 12, 14, 10
GAP = 0.18           # seconds between blocks (consecutive lines of one kind appear together)
HOLD = 3.0           # pause at the end before looping
FONT = "Consolas,'Courier New',monospace"


def build(out):
    n = len(LINES)
    height = TOP + LH * n + PAD
    # Block index per line: a block is a run of lines with the same label kind.
    kinds, groups, g, prev = [], [], -1, None
    for segs in LINES:
        kind = None if segs is None else (segs[2][0] if segs[0][0].startswith("[") else segs[1][0] if len(segs) > 1 else "prompt")
        if kind != prev or kind is None:
            g += 1
        groups.append(g)
        prev = kind
    # The prompt and the first info block are instant.
    times = [max(0.0, (gi - 1)) * GAP for gi in groups]
    total = max(times) + GAP + HOLD
    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{height}" viewBox="0 0 {W} {height}" font-family="{FONT}" font-size="9.5">',
        f'<rect width="{W}" height="{height}" fill="{BG}"/>',
    ]
    for i, segs in enumerate(LINES):
        t = times[i]
        y = TOP + LH * i + 14
        if segs is None:
            continue
        # Self-looping keyframes: hidden until t, visible until the hold ends, then hidden again.
        k1 = t / total
        k2 = min((t + 0.08) / total, 0.999)
        k3 = (total - 0.05) / total
        parts.append(f'<g opacity="0"><animate attributeName="opacity" values="0;0;1;1;0" keyTimes="0;{k1:.4f};{k2:.4f};{k3:.4f};1" dur="{total:.2f}s" repeatCount="indefinite"/>')
        parts.append(f'<text x="{PAD}" y="{y}" xml:space="preserve">')
        for text, colour, bold in segs:
            w = ''
            u = ' text-decoration="underline"' if bold == "u" else ''
            parts.append(f'<tspan fill="{colour}"{w}{u}>{html.escape(text)}</tspan>')
        parts.append('</text></g>')
    # blinking cursor after the last line, only during the hold
    cy = TOP + LH * n + 4
    k = (max(times) + GAP) / total
    parts.append(f'<rect x="{PAD}" y="{cy - 9}" width="6" height="11" fill="{FG}" opacity="0">'
                 f'<animate attributeName="opacity" values="0;0;1;0;1;0;1;0" keyTimes="0;{k:.4f};{k + (1 - k) * 0.15:.4f};{k + (1 - k) * 0.3:.4f};{k + (1 - k) * 0.45:.4f};{k + (1 - k) * 0.6:.4f};{k + (1 - k) * 0.75:.4f};1" dur="{total:.2f}s" repeatCount="indefinite"/></rect>')
    parts.append('</svg>')
    open(out, "w", encoding="utf-8").write("\n".join(parts))
    print(f"wrote {out} ({n} lines, {total:.1f}s loop)")


if __name__ == "__main__":
    build(sys.argv[1] if len(sys.argv) > 1 else "docs/cli.svg")
