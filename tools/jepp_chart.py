#!/usr/bin/env python3
"""Render an amdbgen airport folder as a Jeppesen-style airport diagram (HTML).

Usage: python tools/jepp_chart.py out/KJFK [chart.html]
"""
import json
import os
import sys

LAYERS = [
    "aerodromereferencepoint", "runwayelement", "runwaydisplacedarea", "blastpad", "stopway",
    "taxiwayelement", "apronelement", "verticalpolygonalstructure", "water", "taxiwayguidanceline",
    "runwayexitline", "taxiwayholdingposition", "parkingstandlocation", "runwaythreshold",
    "frequencyarea", "hotspot", "verticalpointstructure", "constructionarea",
]
KEEP = {
    "runwayelement": ["idrwy", "width", "length", "surftype"], "taxiwayelement": ["idlin"],
    "apronelement": ["idapron"], "verticalpolygonalstructure": ["plysttyp", "name"],
    "taxiwayguidanceline": ["idlin"], "runwayexitline": ["idlin"], "taxiwayholdingposition": ["idrwy"],
    "parkingstandlocation": ["idstd"], "runwaythreshold": ["idthr", "idrwy", "brngtrue", "tora", "lda", "width", "rwymktyp"],
    "frequencyarea": ["frq", "station", "name"], "hotspot": ["idhot"], "verticalpointstructure": ["pntsttyp", "name"],
    "aerodromereferencepoint": ["idarpt", "iata", "name", "city", "country", "elev", "lat", "lon", "transalt"],
}


def slim(layer, fc):
    keep = KEEP.get(layer, [])
    return [{"g": f["geometry"], "p": {k: f["properties"].get(k) for k in keep if f["properties"].get(k) is not None}} for f in fc.get("features", [])]


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(1)
    d = sys.argv[1]
    out = sys.argv[2] if len(sys.argv) > 2 else os.path.join(d, "chart.html")
    manifest = json.load(open(os.path.join(d, "manifest.json"), encoding="utf-8"))
    data = {}
    for layer in LAYERS:
        p = os.path.join(d, layer + ".geojson")
        if os.path.exists(p):
            data[layer] = slim(layer, json.load(open(p, encoding="utf-8")))
    tpl = open(os.path.join(os.path.dirname(__file__), "jepp_template.html"), encoding="utf-8").read()
    html = tpl.replace("/*__DATA__*/null", json.dumps({"manifest": manifest, "layers": data}, separators=(",", ":")))
    open(out, "w", encoding="utf-8").write(html)
    print(f"wrote {out} ({len(html) // 1024} KB)")


if __name__ == "__main__":
    main()
