#!/usr/bin/env python3
"""Render an amdbgen airport folder as a self-contained OANS-style HTML viewer.

Usage: python tools/oans_view.py out/EDDF [viewer.html]

The page inlines the layers it draws (runways, taxiways, aprons, buildings, water,
guidance lines, holding positions, stands, thresholds, ARP) so it works offline.
"""
import json
import os
import sys

LAYERS = [
    "aerodromereferencepoint", "runwayelement", "runwaydisplacedarea", "blastpad", "stopway",
    "runwayshoulder", "taxiwayelement", "taxiwayshoulder", "apronelement", "serviceroad",
    "verticalpolygonalstructure", "water", "constructionarea", "deicingarea",
    "taxiwayguidanceline", "standguidanceline", "runwayexitline", "taxiwayholdingposition",
    "taxiwayintersectionmarking", "paintedcenterline", "parkingstandlocation", "runwaythreshold",
    "runwaymarking", "aerodromesign", "hotspot", "bridgeside", "finalapproachandtakeoffarea",
]

KEEP_PROPS = {
    "runwayelement": ["idrwy"], "taxiwayelement": ["idlin"], "apronelement": ["idapron"],
    "verticalpolygonalstructure": ["plysttyp", "name"], "taxiwayguidanceline": ["idlin"],
    "runwayexitline": ["idlin", "idrwy"], "taxiwayholdingposition": ["idrwy", "catstop"],
    "parkingstandlocation": ["idstd", "brngtrue"], "runwaythreshold": ["idthr", "idrwy", "brngtrue", "lda", "tora"],
    "runwaymarking": ["marktype", "text"], "aerodromesign": ["msgfront", "signtype", "signdir"],
    "aerodromereferencepoint": ["idarpt", "name", "iata", "elev"], "hotspot": ["idhot"],
}


def slim(layer, fc):
    keep = KEEP_PROPS.get(layer, [])
    out = []
    for f in fc.get("features", []):
        p = f.get("properties", {})
        out.append({"g": f["geometry"], "p": {k: p.get(k) for k in keep if p.get(k) is not None}})
    return out


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(1)
    d = sys.argv[1]
    out = sys.argv[2] if len(sys.argv) > 2 else os.path.join(d, "viewer.html")
    manifest = json.load(open(os.path.join(d, "manifest.json"), encoding="utf-8"))
    data = {}
    for layer in LAYERS:
        p = os.path.join(d, layer + ".geojson")
        if os.path.exists(p):
            data[layer] = slim(layer, json.load(open(p, encoding="utf-8")))
    tpl = open(os.path.join(os.path.dirname(__file__), "oans_template.html"), encoding="utf-8").read()
    payload = json.dumps({"manifest": manifest, "layers": data}, separators=(",", ":"))
    html = tpl.replace("/*__DATA__*/null", payload)
    open(out, "w", encoding="utf-8").write(html)
    print(f"wrote {out} ({len(html) // 1024} KB)")


if __name__ == "__main__":
    main()
