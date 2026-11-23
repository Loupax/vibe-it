//! W6: paths that bypass the resource handler, switched off or sent to the proxy.
//!
//! The handler (handler.rs) answers every http(s) request a page makes. What it never
//! sees is whatever Chromium's network stack does on its own: background services
//! (component updater, spell-check dictionaries, autofill, safe browsing, ...),
//! speculative DNS/preconnect, QUIC, DNS-over-HTTPS, WebSockets. Inside the empty
//! namespace those fail anyway; here they are switched off so they are not even tried,
//! and the one that must work (WebSockets) goes to proxy.rs, which hands it to wire.
//! Evidence and the full table: web/spike/README.md, "What W6 learned".
//!
//! `WEB_SPIKE_W6=0` leaves all of this out (to compare against W5's behaviour).

use cef::*;

/// Command-line switches (name, value; "" = no value) for the browser process.
pub const SWITCHES: &[(&str, &str)] = &[
    // No QUIC: it is UDP, which wire cannot carry; wire talks HTTP/1.1 and h2 itself.
    ("disable-quic", ""),
    // Chrome's background fetches: variations, component updater, translate ranking,
    // GCM, safe-browsing list updates, ... (the umbrella switch), and the ones it misses.
    ("disable-background-networking", ""),
    ("disable-component-update", ""),
    ("disable-domain-reliability", ""),
    ("disable-sync", ""),
    ("disable-breakpad", ""),
    ("disable-client-side-phishing-detection", ""),
    ("disable-features", "AutofillServerCommunication,OptimizationHints,MediaRouter,NetworkTimeServiceQuerying,DnsOverHttps"),
    // Chromium's own resolver answers NOTFOUND for every name. The handler never needs
    // it (wire resolves); without this the network service still resolves hostnames
    // for preconnects through the host's systemd-resolved (a Unix socket, reachable
    // from the empty namespace; run.sh hides it too). With the proxy set, CONNECT
    // targets are never resolved locally.
    ("host-resolver-rules", "MAP * ~NOTFOUND, EXCLUDE 127.0.0.1"),
];

/// Profile preferences (name, value), set on the global request context.
pub enum Pref {
    Int(i32),
    Bool(bool),
    Str(&'static str),
}

pub const PREFS: &[(&str, Pref)] = &[
    // NETWORK_PREDICTION_NEVER: no DNS prefetch, no preconnect, no search prefetch.
    // Speculation rules / <link rel=prefetch> still go through the handler (W6 checked).
    ("net.network_prediction_options", Pref::Int(2)),
    // Spell-check needs a dictionary download (redirector.gvt1.com) that bypasses the
    // handler; off until the shell fetches the .bdic through wire into the profile.
    ("browser.enable_spellchecking", Pref::Bool(false)),
    ("spellcheck.use_spelling_service", Pref::Bool(false)),
    ("safebrowsing.enabled", Pref::Bool(false)),
    ("search.suggest_enabled", Pref::Bool(false)),
    ("alternate_error_pages.enabled", Pref::Bool(false)),
    ("translate.enabled", Pref::Bool(false)),
    ("url_keyed_anonymized_data_collection.enabled", Pref::Bool(false)),
    // WebRTC: never UDP outside a proxy (there is no UDP proxy, so no UDP at all) and no
    // host candidates; with the proxy set it may only try TURN over TCP, which proxy.rs
    // refuses (not HTTP). A known loss, but it cannot leak.
    ("webrtc.ip_handling_policy", Pref::Str("disable_non_proxied_udp")),
];

/// Local-state (browser-wide) preferences.
pub const GLOBAL_PREFS: &[(&str, Pref)] = &[("dns_over_https.mode", Pref::Str("off"))];

pub fn enabled() -> bool {
    std::env::var("WEB_SPIKE_W6").map(|v| v != "0").unwrap_or(true)
}

/// Adds SWITCHES plus the proxy switches (`proxy`: port and SPKI hash).
pub fn add_switches(cmd: &mut CommandLine, proxy: Option<(u16, Option<String>)>) {
    // Experiments: WEB_SPIKE_W6_SKIP=name,name leaves those switches/prefs out.
    let skip = std::env::var("WEB_SPIKE_W6_SKIP").unwrap_or_default();
    for (name, value) in SWITCHES {
        if skip.split(',').any(|s| s == *name) {
            continue;
        }
        let n = CefString::from(*name);
        if *name == "disable-features" {
            // Merge, never replace: CEF puts its own list here before we are called, and
            // replacing it crashes the Chrome UI at the first tab (SIGSEGV in
            // tabs::TabInterface::GetFromContents, seen in W6).
            let old = CefString::from(&cmd.switch_value(Some(&n))).to_string();
            let merged = if old.is_empty() { value.to_string() } else { format!("{old},{value}") };
            log!("disable-features: had {old:?}, now {merged:?}");
            cmd.append_switch_with_value(Some(&n), Some(&CefString::from(merged.as_str())));
        } else if value.is_empty() {
            cmd.append_switch(Some(&n));
        } else {
            cmd.append_switch_with_value(Some(&n), Some(&CefString::from(*value)));
        }
    }
    if let Some((port, spki)) = proxy {
        let set = |n: &str, v: &str| cmd.append_switch_with_value(Some(&CefString::from(n)), Some(&CefString::from(v)));
        set("proxy-server", &format!("http://127.0.0.1:{port}"));
        // Chromium bypasses a proxy for loopback targets by default; `<-loopback>` takes
        // that out, so ws://localhost/ also goes to wire (there is nothing else on this
        // namespace's loopback but the proxy).
        set("proxy-bypass-list", "<-loopback>");
        if let Some(spki) = spki {
            set("ignore-certificate-errors-spki-list", &spki);
        }
    }
}

fn value(p: &Pref) -> Option<Value> {
    let v = value_create()?;
    match p {
        Pref::Int(i) => v.set_int(*i),
        Pref::Bool(b) => v.set_bool(*b as i32),
        Pref::Str(s) => v.set_string(Some(&CefString::from(*s))),
    };
    Some(v)
}

fn set_all(pm: &impl ImplPreferenceManager, prefs: &[(&str, Pref)], what: &str) {
    let skip = std::env::var("WEB_SPIKE_W6_SKIP").unwrap_or_default();
    for (name, p) in prefs {
        if skip.split(',').any(|s| s == *name || s == "prefs") {
            continue;
        }
        let n = CefString::from(*name);
        // A non-empty string: an empty CefString is a null pointer, and CEF then drops the error text.
        let mut err = CefString::from("-");
        let ok = match value(p) {
            Some(mut v) => pm.set_preference(Some(&n), Some(&mut v), Some(&mut err)) == 1,
            None => false,
        };
        if !ok {
            log!(
                "{what} pref {name} NOT set (has {}, can set {}): {err}",
                pm.has_preference(Some(&n)),
                pm.can_set_preference(Some(&n))
            );
        }
    }
}

/// Sets PREFS on the global request context and GLOBAL_PREFS on local state
/// (UI thread, after the context is initialized).
pub fn set_prefs() {
    if let Some(ctx) = request_context_get_global_context() {
        set_all(&ctx, PREFS, "profile");
    }
    if let Some(pm) = preference_manager_get_global() {
        set_all(&pm, GLOBAL_PREFS, "local-state");
    }
    log!("W6: {} switches, {} profile prefs, {} local-state prefs applied", SWITCHES.len(), PREFS.len(), GLOBAL_PREFS.len());
}
