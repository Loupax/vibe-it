//! QY39b: the `--user-agent <string>` override.
//!
//! scratchy's default is to send nothing: wire then uses its own honest
//! `User-Agent`. With `--user-agent X` (or `SCRATCHY_USER_AGENT=X`; the flag
//! wins) every request built for wire carries `X-Wire-User-Agent: X`, which a
//! wire with the QY39a feature (`X-Wire-Features: ua`) sends upstream as the
//! `User-Agent`. An older wire never advertises the feature: the first
//! response teaches scratchy, ONE warning line is logged and wire's default
//! applies (the header is no longer sent). `navigator.userAgent` and
//! `navigator.appVersion` report the same string, so JS and the wire agree.
//!
//! The header is added in exactly two places, the two request builders
//! (`build_get_request_text_with`, `build_wire_request_with`), so page loads,
//! sub-resources, fetch/XHR/sendBeacon and media requests all get it once.
//! No named presets (`firefox`/`chrome`): any such string goes stale, and the
//! user can pass the full string.

use std::io;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering::Relaxed};
use std::sync::OnceLock;

/// The wire request header (QY39a).
pub(crate) const WIRE_HEADER_USER_AGENT: &str = "X-Wire-User-Agent";
/// Same limit as wire's `bad-user-agent` check.
pub(crate) const MAX_USER_AGENT_BYTES: usize = 512;
/// Environment variable with the same meaning as the flag (the flag wins).
pub(crate) const USER_AGENT_ENV: &str = "SCRATCHY_USER_AGENT";
/// Logged once when the flag is set but wire cannot honour it.
pub(crate) const OLD_WIRE_WARNING: &str = "scratchy: wire does not support a User-Agent override (restart wire from a current build)";

/// Same rules as wire: non-empty, printable ASCII (space allowed), <= 512 bytes.
pub(crate) fn validate_user_agent(s: &str) -> Result<(), String> {
    if s.is_empty() {
        return Err("the User-Agent is empty".to_string());
    }
    if s.len() > MAX_USER_AGENT_BYTES {
        return Err(format!("the User-Agent is {} bytes, the limit is {MAX_USER_AGENT_BYTES}", s.len()));
    }
    if let Some(c) = s.chars().find(|c| !(' '..='~').contains(c)) {
        return Err(format!("the User-Agent must be printable ASCII (found {c:?})"));
    }
    Ok(())
}

/// The override plus what wire said about supporting it.
pub(crate) struct UaOverride {
    value: Option<String>,
    /// 0 unknown (send it), 1 wire advertised `ua`, 2 wire did not.
    state: AtomicU8,
    warned: AtomicBool,
}

impl UaOverride {
    pub(crate) fn new(value: Option<String>) -> Self {
        UaOverride { value, state: AtomicU8::new(0), warned: AtomicBool::new(false) }
    }

    pub(crate) fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }

    /// The header to add to a request: `None` without the flag, or once wire
    /// proved unable to honour it.
    pub(crate) fn header(&self) -> Option<(String, String)> {
        let v = self.value.as_ref()?;
        (self.state.load(Relaxed) != 2).then(|| (WIRE_HEADER_USER_AGENT.to_string(), v.clone()))
    }

    /// Feeds one wire response's `X-Wire-Features` value. Returns the warning
    /// line, once, the first time wire turns out not to support the override.
    pub(crate) fn observe(&self, features: Option<&str>) -> Option<&'static str> {
        self.value.as_ref()?;
        let has_ua = features.is_some_and(|f| f.split(',').any(|t| t.trim().eq_ignore_ascii_case("ua")));
        if has_ua {
            self.state.store(1, Relaxed);
            return None;
        }
        self.state.store(2, Relaxed);
        (!self.warned.swap(true, Relaxed)).then_some(OLD_WIRE_WARNING)
    }
}

static UA: OnceLock<UaOverride> = OnceLock::new();

/// Installs the process-wide override (startup only; a second call is ignored).
pub(crate) fn set_user_agent_override(value: Option<String>) {
    let _ = UA.set(UaOverride::new(value));
}

pub(crate) fn user_agent_state() -> &'static UaOverride {
    UA.get_or_init(|| UaOverride::new(None))
}

/// Records a response's `X-Wire-Features` (logs the old-wire warning once).
pub(crate) fn note_ua_response(features: Option<&str>) {
    if let Some(line) = user_agent_state().observe(features) {
        diag!("{line}");
    }
}

/// `navigator.userAgent`: the override when set, else scratchy's own string.
pub(crate) fn navigator_user_agent() -> String {
    navigator_user_agent_for(user_agent_state().value())
}

pub(crate) fn navigator_user_agent_for(over: Option<&str>) -> String {
    match over {
        Some(v) => v.to_string(),
        None => format!("Mozilla/5.0 (X11; Linux x86_64) scratchy/{}", env!("CARGO_PKG_VERSION")),
    }
}

/// `navigator.appVersion`: the UA minus its `Mozilla/` prefix (what browsers
/// do); today's fixed value when there is no override.
pub(crate) fn navigator_app_version_for(over: Option<&str>) -> String {
    match over {
        Some(v) => v.strip_prefix("Mozilla/").unwrap_or(v).to_string(),
        None => "5.0 (X11)".to_string(),
    }
}

pub(crate) fn navigator_app_version() -> String {
    navigator_app_version_for(user_agent_state().value())
}

/// Finds `--user-agent <value>` in the arguments. `Ok(None)` when absent.
pub(crate) fn user_agent_from_args(args: &[String]) -> Result<Option<String>, String> {
    match args.iter().position(|a| a == "--user-agent") {
        None => Ok(None),
        Some(i) => args.get(i + 1).cloned().map(Some).ok_or_else(|| "--user-agent needs a string argument".to_string()),
    }
}

/// Flag, else environment variable, validated. An empty variable is unset.
pub(crate) fn resolve_user_agent(args: &[String], env_value: Option<String>) -> Result<Option<String>, String> {
    let chosen = match user_agent_from_args(args)? {
        Some(v) => Some(v),
        None => env_value.filter(|v| !v.is_empty()),
    };
    if let Some(v) = &chosen {
        validate_user_agent(v)?;
    }
    Ok(chosen)
}

/// Startup: resolve, install, or exit 1 with a clear message.
pub(crate) fn init_from_process(args: &[String]) {
    match resolve_user_agent(args, std::env::var(USER_AGENT_ENV).ok()) {
        Ok(v) => set_user_agent_override(v),
        Err(e) => {
            diag!("scratchy: invalid User-Agent: {e}");
            std::process::exit(1);
        }
    }
}
