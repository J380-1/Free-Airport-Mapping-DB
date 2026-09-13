"""Port the GM5 A220 Airport Moving Map (built for MSFS 2024) to the MSFS 2020 Synaptic
A220 v1.0.8 and make it work with amdb-bridge without a Navigraph login.

Changes:
* package renamed zzz-gm5-a220-amm so it loads after synaptic-aircraft-a220 (MSFS 2020
  applies Community packages alphabetically; the later one wins for the same path);
* manifest: 2020 game version, no 2024-only fields;
* gm5-a220-amm.js:
  - when the A220 has no stored Navigraph token, use a placeholder so the map still
    requests AMDB data (the bridge ignores bearer tokens);
  - when the sim's nearest-airport search returns nothing (or fails), ask the bridge
    (/v1/nearest) for the airports around the aircraft instead;
* layout.json regenerated.

Normally not needed: `amdb-bridge serve` applies the same script changes in place.
This manual version also renames the package for MSFS 2020 and rewrites its manifest.

Usage: python tools/port_a220_amm.py <unzipped gm5-a220-amm folder> <Community folder>
"""
import json, os, shutil, sys, time

if len(sys.argv) < 3:
    print(__doc__)
    sys.exit(1)
SRC = sys.argv[1]
DST_ROOT = sys.argv[2]
DST = os.path.join(DST_ROOT, "zzz-gm5-a220-amm")

if os.path.isdir(DST):
    shutil.rmtree(DST)
shutil.copytree(SRC, DST)

# manifest for MSFS 2020
mp = os.path.join(DST, "manifest.json")
m = json.load(open(mp, encoding="utf-8"))
m["title"] = "GM5 A220 Airport Moving Map (MSFS 2020 port, amdb-bridge)"
m["minimum_game_version"] = "1.30.0"
m.pop("builder", None)
m.pop("minimum_compatibility_version", None)
m["release_notes"] = {"neutral": {"LastUpdate": "Ported to MSFS 2020 / Synaptic A220 1.0.8; works with amdb-bridge without a Navigraph subscription.", "OlderHistory": ""}}
json.dump(m, open(mp, "w", encoding="utf-8"), indent=2)

jp = os.path.join(DST, "html_ui", "Pages", "VCockpit", "Instruments", "a22x", "DisplayUnits", "gm5-a220-amm.js")
js = open(jp, encoding="utf-8").read()

def rw(old, new, what):
    global js
    assert old in js, what + " not found"
    js = js.replace(old, new, 1)

# 1. token fallback
rw("""    function currentStoredAccessToken() {
        const token = normalizeToken(readStoredValue(DU_ACCESS_KEY));
        if (!token) return null;
        const info = inspectToken(token);
        return info.expired ? null : token;
    }""", """    function currentStoredAccessToken() {
        /* amdb-bridge: no Navigraph login needed; the local bridge ignores bearer tokens. */
        const token = normalizeToken(readStoredValue(DU_ACCESS_KEY));
        if (!token) return 'amdb-bridge-local';
        const info = inspectToken(token);
        return info.expired ? 'amdb-bridge-local' : token;
    }""", "token function")

# 2. nearest-airport fallback through the bridge
rw("""    async function runNearestSearchWithRetry(state, radiusMeters, maxItems, stage) {""",
"""    /* amdb-bridge: the sim's facility search is not always available (MSFS 2020 port);
       when it fails or returns nothing, ask the bridge for the airports around us and
       register them as ready-made facilities so no LOAD_AIRPORT round trip is needed. */
    function bridgeFacilityKey(ident) {
        return 'A      ' + String(ident).toUpperCase() + ' ';
    }

    async function bridgeNearestSearch(state, radiusMeters, maxItems, stage) {
        const url = `${AMDB_BASE}/nearest?lat=${encodeURIComponent(state.lat)}&lon=${encodeURIComponent(state.lon)}&radius_km=${Math.max(5, Math.round(radiusMeters / 1000))}&limit=${maxItems || 16}`;
        const response = await withTimeout(fetch(url, { method: 'GET', headers: { 'Accept': 'application/json' } }), SEARCH_TIMEOUT_MS, `bridge nearest ${stage || ''} timeout`);
        if (!response.ok) throw new Error(`bridge nearest HTTP ${response.status}`);
        const rows = await response.json();
        const added = [];
        if (nearest && Array.isArray(rows)) {
            for (let i = 0; i < rows.length; i++) {
                const row = rows[i] || {};
                const ident = String(row.idarpt || '').trim().toUpperCase();
                if (!/^[A-Z]{4}$/.test(ident)) continue;
                const coords = row.coordinates || {};
                const lat = Number(coords.lat), lon = Number(coords.lon);
                if (!Number.isFinite(lat) || !Number.isFinite(lon)) continue;
                const key = bridgeFacilityKey(ident);
                nearest.facilities.set(key, { icao: key, ident: ident, name: row.name || ident, lat: lat, lon: lon, runways: [], source: 'amdb-bridge' });
                added.push(key);
            }
        }
        log('Bridge nearest airports', added.length, stage || '');
        return { sessionId: nearest ? nearest.sessionId : null, searchId: 'bridge', added: added, removed: [] };
    }

    async function runNearestSearchWithRetry(state, radiusMeters, maxItems, stage) {
        let native = null;
        let nativeError = null;
        try {
            native = await runNearestSearchWithRetryNative(state, radiusMeters, maxItems, stage);
        } catch (e) {
            nativeError = e;
        }
        const nativeCount = native && Array.isArray(native.added) ? native.added.length : 0;
        const known = nearest && nearest.currentRaw ? nearest.currentRaw.size : 0;
        if (nativeCount > 0 || (known > 0 && !nativeError)) return native;
        try {
            return await bridgeNearestSearch(state, radiusMeters, maxItems, stage);
        } catch (e) {
            if (nativeError) throw nativeError;
            throw e;
        }
    }

    async function runNearestSearchWithRetryNative(state, radiusMeters, maxItems, stage) {""", "nearest search")

open(jp, "w", encoding="utf-8", newline="\n").write(js)

# layout.json
ft = int((time.time() + 11644473600) * 1e7)
content = []
for root, _, files in os.walk(DST):
    for f in files:
        p = os.path.join(root, f)
        rel = os.path.relpath(p, DST).replace("\\", "/")
        if rel in ("layout.json", "manifest.json"):
            continue
        content.append({"path": rel.lower(), "size": os.path.getsize(p), "date": ft})
content.sort(key=lambda c: c["path"])
json.dump({"content": content}, open(os.path.join(DST, "layout.json"), "w", encoding="utf-8"), indent=2)
print("ported to", DST)
for c in content:
    print("  ", c["path"], c["size"])
