'use strict';
/* global Coherent, RegisterViewListener */

// AMDB token shim for encrypted iniBuilds aircraft (e.g. the Marketplace A380).
//
// Their EFB files are sealed, so the bridge cannot rewrite the token handler the way
// it does for Community installs. This panel does the same job over the comm bus
// while it is open: it answers the OANS gauge's `RequestNavigraphAccessToken` with the
// bridge placeholder and re-pushes it periodically, so the aircraft's OWN airport map
// draws from amdb-bridge. No Navigraph account, no hosts-file redirect for this path
// beyond what `serve` already sets up, and no aircraft files are touched.
//
// Verification: trigger the airport map, then look for `(with token)` next to your
// aircraft in the bridge log. If requests arrive without a token, this panel was not
// open (panels only run while open) or the gauge uses different event names.

(function () {
    var TOKEN = 'amdb-bridge-local';
    var REQ = 'RequestNavigraphAccessToken';
    var SET = 'SetNavigraphAccessToken';
    var RETRY_MS = 2000;
    var PUSH_MS = 30000;

    var pushes = 0;
    var answers = 0;
    var listening = false;
    var statusEl = null;

    function note(text) {
        if (statusEl) statusEl.textContent = text;
        if (typeof console !== 'undefined' && console.log) console.log('[amdb-shim] ' + text);
    }

    // The push the patched EFB performs: hand the gauge a token it will accept.
    // The bridge ignores bearer tokens; it only needs the gauge to believe it has one.
    function push() {
        try {
            Coherent.call('COMM_BUS_WASM_CALLBACK', SET, TOKEN);
            pushes++;
            note('token pushed x' + pushes + ' / requests answered x' + answers);
            return true;
        } catch (e) {
            note('push failed: ' + e);
            return false;
        }
    }

    // Answer token requests the way the rewritten EFB handler does.
    function listen() {
        if (listening) return;
        try {
            if (typeof RegisterViewListener !== 'function') {
                note('sim not ready, retrying…');
                setTimeout(listen, RETRY_MS);
                return;
            }
            var listener = RegisterViewListener('JS_LISTENER_COMM_BUS');
            listener.on(REQ, function () {
                answers++;
                push();
            });
            listening = true;
            note('listening for token requests');
        } catch (e) {
            note('listen failed, retrying: ' + e);
            setTimeout(listen, RETRY_MS);
        }
    }

    function start() {
        statusEl = document.getElementById('amdbshim-status');
        var btn = document.getElementById('amdbshim-push');
        if (btn && btn.addEventListener) {
            btn.addEventListener('click', function () { push(); });
        }
        listen();
        push();
        setInterval(push, PUSH_MS);
    }

    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', start);
    } else {
        start();
    }
})();
