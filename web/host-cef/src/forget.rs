//! `forget-site` (STASH.md "Forget site", PROTOCOL.md "forget-site"), the
//! engine-neutral part: the queue of requests, the record of origins the page loaded,
//! and the `forgot` answer. What an engine must delete, and how, is behind
//! [`SiteForgetter`]; web-host-cef's implementation is `forget_cef.rs`. A
//! web-host-webkit (E1) would implement the same trait with
//! `webkit_website_data_manager_fetch` / `_remove` (PROTOCOL.md).

use crate::control::emit;
use crate::protocol::{self, Event, ForgetResult};
use cef::quit_message_loop;
use std::cell::RefCell;
use std::collections::{BTreeSet, VecDeque};
use std::io::Write;
use std::sync::{Mutex, OnceLock};

/// One `forget-site`: what the shell sent and what it means.
#[derive(Debug, Clone)]
pub struct ForgetJob {
    /// The argument as sent (echoed in `forgot`).
    pub site: String,
    /// The registrable domain: it and every host under it are forgotten.
    pub domain: String,
    /// Origins under the site that the host's page loaded (from the record). An engine
    /// that can list what it stores (WebKitGTK) may ignore it.
    pub recorded: Vec<String>,
}

/// What an engine did for one job.
#[derive(Debug, Clone, Default)]
pub struct ForgetReport {
    /// Cookies deleted.
    pub cookies: u32,
    /// Origins (or, for engines that group by domain, data records) cleared.
    pub origins: Vec<String>,
    /// What could not be cleared (`cookies`, `http-cache`, `storage <origin> (<why>)`).
    pub failed: Vec<String>,
}

/// The contract every engine host implements for `forget-site`. Runs on the host's
/// UI thread. `start` deletes, for every host under `job.domain` over http and
/// https: cookies, localStorage, sessionStorage (where reachable), IndexedDB, Cache
/// Storage, service workers, other per-origin storage, and the site's HTTP cache
/// entries (or all of them if the engine cannot select), flushes to disk, then calls
/// [`finished`] exactly once. One job at a time: the queue is here.
pub trait SiteForgetter {
    fn start(&self, job: ForgetJob);
    /// Release what the forgetter holds (the host is quitting or idle), then call
    /// `then` (at once if nothing is held).
    fn shutdown(&self, then: Box<dyn FnOnce()>);
}

/// `<profile>/web-host-origins`, set in main.
pub static RECORD: OnceLock<String> = OnceLock::new();
/// `--no-window`: quit after the first forget; its exit code.
pub static ONE_SHOT: OnceLock<bool> = OnceLock::new();
pub static EXIT_CODE: Mutex<i32> = Mutex::new(0);

#[derive(Default)]
struct State {
    engine: Option<Box<dyn SiteForgetter>>,
    queue: VecDeque<ForgetJob>,
    running: Option<ForgetJob>,
    recorded: Option<BTreeSet<String>>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

/// Installs the engine's implementation (main, before the message loop).
pub fn set_engine(engine: Box<dyn SiteForgetter>) {
    with(|s| s.engine = Some(engine));
}

fn one_shot() -> bool {
    *ONE_SHOT.get().unwrap_or(&false)
}

fn load_record(s: &mut State) -> &mut BTreeSet<String> {
    s.recorded.get_or_insert_with(|| {
        RECORD
            .get()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| t.lines().filter(|l| !l.is_empty()).map(String::from).collect())
            .unwrap_or_default()
    })
}

/// Records the origin of a URL the page loaded (any frame). The engine cannot list
/// the origins that have storage (CEF), so this is where forget-site finds them.
pub fn record(url: &str) {
    let Some((origin, _)) = protocol::origin_of(url) else { return };
    let new = with(|s| load_record(s).insert(origin.clone()));
    if !new {
        return;
    }
    let Some(path) = RECORD.get() else { return };
    match std::fs::OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut f) => {
            use std::os::unix::fs::PermissionsExt;
            let _ = f.set_permissions(std::fs::Permissions::from_mode(0o600));
            let _ = f.write_all(format!("{origin}\n").as_bytes());
        }
        Err(e) => log!("cannot record origin {origin} in {path}: {e}"),
    }
}

/// Drops the forgotten origins from the record (it names visited sites).
fn unrecord(domain: &str) {
    let kept: Vec<String> = with(|s| {
        let rec = load_record(s);
        rec.retain(|o| {
            protocol::origin_of(o)
                .map(|(_, h)| !protocol::host_in_site(&h, domain))
                .unwrap_or(false)
        });
        rec.iter().cloned().collect()
    });
    let Some(path) = RECORD.get() else { return };
    let tmp = format!("{path}.tmp");
    let body: String = kept.iter().map(|o| format!("{o}\n")).collect();
    if let Err(e) = std::fs::write(&tmp, body).and_then(|_| std::fs::rename(&tmp, path)) {
        log!("cannot rewrite the origin record {path}: {e}");
    }
}

/// `forget-site <site>` (UI thread): runs now, or after the forget in progress.
pub fn forget_site(site: String) {
    let domain = match protocol::forget_domain(&site) {
        Ok(d) => d,
        Err(e) => {
            emit(Event::Error {
                message: format!("forget-site: {e}"),
            });
            if one_shot() {
                *EXIT_CODE.lock().unwrap() = 2;
                quit_message_loop();
            }
            return;
        }
    };
    let busy = with(|s| {
        let recorded = load_record(s)
            .iter()
            .filter(|o| {
                protocol::origin_of(o)
                    .map(|(_, h)| protocol::host_in_site(&h, &domain))
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
        s.queue.push_back(ForgetJob { site, domain, recorded });
        s.running.is_some()
    });
    if !busy {
        start_next();
    }
}

fn start_next() {
    let job = with(|s| {
        let job = s.queue.pop_front()?;
        s.running = Some(job.clone());
        Some(job)
    });
    let Some(job) = job else { return };
    log!("forget-site {}: domain {} and every host under it", job.site, job.domain);
    // The engine is taken out while it runs, so it may call back into this module.
    let engine = with(|s| s.engine.take());
    match &engine {
        Some(e) => e.start(job),
        None => finished(ForgetReport {
            failed: vec!["no engine".into()],
            ..Default::default()
        }),
    }
    with(|s| {
        if s.engine.is_none() {
            s.engine = engine;
        }
    });
}

/// The engine is done with the running job.
pub fn finished(report: ForgetReport) {
    let Some(job) = with(|s| s.running.take()) else { return };
    unrecord(&job.domain);
    let result = if report.failed.is_empty() {
        ForgetResult::Ok
    } else {
        ForgetResult::Partial
    };
    let ev = Event::Forgot {
        site: job.site,
        domain: job.domain,
        result,
        failed: report.failed,
        cookies: report.cookies,
        origins: report.origins,
    };
    if one_shot() {
        println!("{}", protocol::encode_event(&ev));
    }
    emit(ev);
    if with(|s| !s.queue.is_empty()) {
        return start_next();
    }
    let then: Box<dyn FnOnce()> = if one_shot() {
        *EXIT_CODE.lock().unwrap() = if result == ForgetResult::Ok { 0 } else { 1 };
        Box::new(quit_message_loop)
    } else {
        Box::new(|| {})
    };
    shutdown(then);
}

/// Releases the engine's resources (on quit, and when idle).
pub fn shutdown(then: Box<dyn FnOnce()>) {
    let engine = with(|s| s.engine.take());
    match &engine {
        Some(e) => e.shutdown(then),
        None => then(),
    }
    with(|s| {
        if s.engine.is_none() {
            s.engine = engine;
        }
    });
}
