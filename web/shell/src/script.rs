//! C-SCRIPT: the user's init.js (SCRIPTING.md; DECISIONS.md 2026-11-24 "User scripts").
//!
//! QuickJS (rquickjs) with no `std`/`os` modules, no module loader and no timers. Every file is
//! evaluated as one classic script in strict mode, all in one context. The only thing a script sees
//! is the global `web`: `web.cmd`, `web.command`, `web.map` (C-SCRIPT-MAP), `web.on("ready")`.
//!
//! Re-entrancy: no JS function ever holds a borrow of the shell. `web.cmd(line)` only appends the
//! line to a queue (a `RefCell` borrowed for the push and nothing else); after each call into JS
//! returns, `Keys` takes the lines that call queued and runs them in order, one `:` line at a time.
//! A line naming a user command calls into JS again from there (the previous call has returned), so
//! the order is the same as if `web.cmd` had run the line at once: `cmd("A"); cmd("B")` where `:A`
//! does `cmd("C")` runs A, C, B. The nesting of user commands through `web.cmd` is counted
//! (`SCRIPT_DEPTH_MAX`); past it the whole chain stops with an error.
//!
//! The loader takes a list of files (`init_files`), so C-PROFILE's profile script is one more entry.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use rquickjs::context::EvalOptions;
use rquickjs::{Context, Ctx, Exception, Function, Object, Persistent, Runtime, Value};

use crate::config::{SCRIPT_BUDGET_MS, SCRIPT_DEPTH_MAX, SCRIPT_MEMORY_MAX};
use crate::keys::{parse_map_keys, parse_map_rhs, resolve_mapping, Effect, Keys, MapKey, MapLine, MapStep};

/// One script to load: where it is and the name errors give it (`init.js`, or a profile's path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptFile {
    pub path: PathBuf,
    pub label: String,
}

/// The scripts web loads at start, in order: `$XDG_CONFIG_HOME/web/init.js` (default
/// `~/.config/web/init.js`), then with `--profile NAME` `.../web/profiles/NAME/init.js`
/// (its errors are labelled with that path).
pub fn init_files(env: &dyn Fn(&str) -> Option<String>, profile: Option<&str>) -> Vec<ScriptFile> {
    let set = |k: &str| env(k).filter(|v| !v.is_empty());
    let config = match (set("XDG_CONFIG_HOME"), set("HOME")) {
        (Some(x), _) if Path::new(&x).is_absolute() => PathBuf::from(x),
        (_, Some(h)) => Path::new(&h).join(".config"),
        _ => return Vec::new(),
    };
    let mut files = vec![ScriptFile {
        path: config.join("web").join("init.js"),
        label: "init.js".to_string(),
    }];
    if let Some(name) = profile {
        let path = config.join("web").join("profiles").join(name).join("init.js");
        files.push(ScriptFile {
            label: path.display().to_string(),
            path,
        });
    }
    files
}

/// An error out of a script: what the status bar shows (`init.js:LINE: message`) and the full
/// text for the log (with the stack).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptError {
    pub bar: String,
    pub full: String,
}

/// What one call into JS left behind: the `web.cmd` lines it queued, in order, and its error.
#[derive(Debug, Default)]
pub struct CallResult {
    pub lines: Vec<String>,
    pub error: Option<ScriptError>,
}

/// C-SCRIPT-MAP: what a mapping does (SCRIPTING.md "web.map").
#[derive(Clone)]
pub enum MapRhs {
    /// A `:` line: runs with `<CR>`, else opens the `:` line with it typed.
    Line(MapLine),
    /// A function, called with no arguments.
    Func(Persistent<Function<'static>>),
}

/// One `web.map("n", keys, rhs)`: the keys as written (for the log) and parsed.
struct Mapping {
    notation: String,
    keys: Vec<MapKey>,
    rhs: MapRhs,
}

/// What the scripts registered (shared with the `web` functions; borrowed only for a push or a lookup).
#[derive(Default)]
struct Registry {
    queue: Vec<String>,
    commands: BTreeMap<String, Persistent<Function<'static>>>,
    ready: Vec<Persistent<Function<'static>>>,
    /// NORMAL-mode mappings, in the order made (mapping the same keys again replaces in place).
    maps: Vec<Mapping>,
}

/// The QuickJS runtime and its one context.
pub struct Scripts {
    reg: Rc<RefCell<Registry>>,
    deadline: Rc<Cell<Option<Instant>>>,
    interrupted: Rc<Cell<bool>>,
    /// Promises rejected with no handler yet: (identity, message, stack). One that gets a handler
    /// before the call ends is removed; what is left is reported.
    rejected: Rc<RefCell<Vec<(usize, String, String)>>>,
    budget: Duration,
    /// The label errors without a script frame get.
    fallback: String,
    /// Files evaluated so far.
    files: usize,
    /// `web.cmd` lines queued by the files themselves: run when `ready` fires (before the
    /// handlers), since before that there is no host to send commands to.
    held: Vec<String>,
    ready_fired: bool,
    /// User commands nested through `web.cmd` right now, and whether the chain hit the limit.
    depth: usize,
    overflow: bool,
    ctx: Context,
    rt: Runtime,
}

impl Drop for Scripts {
    fn drop(&mut self) {
        // Persistent values must go before the runtime does.
        let mut r = self.reg.borrow_mut();
        r.commands.clear();
        r.ready.clear();
        r.maps.clear();
    }
}

/// `web.command`'s name rule: an uppercase first letter (so a user command never shadows a
/// built-in one) and no whitespace (it must be typeable as one word).
pub fn valid_command_name(name: &str) -> bool {
    name.chars().next().is_some_and(char::is_uppercase) && !name.chars().any(char::is_whitespace)
}

impl Scripts {
    /// A runtime with `web` installed and nothing else beyond the language itself.
    pub fn new(budget: Duration) -> Result<Scripts, String> {
        let rt = Runtime::new().map_err(|e| format!("script runtime: {e}"))?;
        let ctx = Context::full(&rt).map_err(|e| format!("script context: {e}"))?;
        let deadline: Rc<Cell<Option<Instant>>> = Rc::default();
        let interrupted: Rc<Cell<bool>> = Rc::default();
        {
            let (deadline, interrupted) = (deadline.clone(), interrupted.clone());
            rt.set_interrupt_handler(Some(Box::new(move || {
                let over = deadline.get().is_some_and(|d| Instant::now() >= d);
                if over {
                    interrupted.set(true);
                }
                over
            })));
        }
        // C-SCRIPT-MEM: an allocation loop ends with QuickJS's out-of-memory error instead of growing.
        rt.set_memory_limit(SCRIPT_MEMORY_MAX);
        let rejected: Rc<RefCell<Vec<(usize, String, String)>>> = Rc::default();
        {
            let rejected = rejected.clone();
            rt.set_host_promise_rejection_tracker(Some(Box::new(move |_ctx, promise, reason, handled| {
                let id = promise_id(&promise);
                let mut r = rejected.borrow_mut();
                if handled {
                    r.retain(|(i, _, _)| *i != id);
                } else {
                    let (message, stack) = message_stack(&reason);
                    r.push((id, message, stack));
                }
            })));
        }
        let reg: Rc<RefCell<Registry>> = Rc::default();
        ctx.with(|ctx| install_web(&ctx, &reg)).map_err(|e| format!("script setup: {e}"))?;
        Ok(Scripts {
            reg,
            deadline,
            interrupted,
            rejected,
            budget,
            fallback: "init.js".to_string(),
            files: 0,
            held: Vec::new(),
            ready_fired: false,
            depth: 0,
            overflow: false,
            ctx,
            rt,
        })
    }

    /// Runs one file (a classic script, strict). What it set up before an error stays.
    pub fn eval_file(&mut self, label: &str, source: &str) -> CallResult {
        if self.files == 0 {
            self.fallback = label.to_string();
        }
        self.files += 1;
        let mut opts = EvalOptions::default();
        opts.global = true;
        opts.strict = true;
        opts.filename = Some(label.to_string());
        let source = source.to_string();
        self.call(label, move |ctx| ctx.eval_with_options::<Value, _>(source, opts).map(|_| ()))
    }

    /// The user commands' names, sorted.
    pub fn command_names(&self) -> Vec<String> {
        self.reg.borrow().commands.keys().cloned().collect()
    }

    pub fn has_command(&self, name: &str) -> bool {
        self.reg.borrow().commands.contains_key(name)
    }

    /// Calls `:name`'s function with `args` as one array of strings.
    pub fn call_command(&mut self, name: &str, args: &[&str]) -> CallResult {
        let Some(f) = self.reg.borrow().commands.get(name).cloned() else {
            return CallResult::default();
        };
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let label = self.fallback.clone();
        self.call(&label, move |ctx| f.restore(ctx)?.call::<_, Value>((args,)).map(|_| ()))
    }

    /// C-SCRIPT-MAP: whether any NORMAL mapping exists (with none, keys go straight to the bindings).
    pub fn has_maps(&self) -> bool {
        !self.reg.borrow().maps.is_empty()
    }

    /// `resolve_mapping` over the mappings.
    pub fn resolve_map(&self, typed: &[Option<MapKey>], can_wait: bool) -> MapStep {
        let r = self.reg.borrow();
        resolve_mapping(typed, r.maps.iter().map(|m| m.keys.as_slice()), can_wait)
    }

    /// Mapping `index`'s keys (as written) and what it does.
    pub fn map_rhs(&self, index: usize) -> Option<(String, MapRhs)> {
        self.reg.borrow().maps.get(index).map(|m| (m.notation.clone(), m.rhs.clone()))
    }

    /// Calls a mapping's function (no arguments).
    pub fn call_map(&mut self, f: Persistent<Function<'static>>) -> CallResult {
        let label = self.fallback.clone();
        self.call(&label, move |ctx| f.restore(ctx)?.call::<_, Value>(()).map(|_| ()))
    }

    /// The `ready` handlers in order (the event fires once: later calls give nothing).
    pub fn take_ready(&mut self) -> (Vec<String>, Vec<Persistent<Function<'static>>>) {
        if self.ready_fired {
            return (Vec::new(), Vec::new());
        }
        self.ready_fired = true;
        (std::mem::take(&mut self.held), self.reg.borrow().ready.clone())
    }

    pub fn call_ready(&mut self, f: Persistent<Function<'static>>) -> CallResult {
        let label = self.fallback.clone();
        self.call(&label, move |ctx| f.restore(ctx)?.call::<_, Value>(()).map(|_| ()))
    }

    /// One call into JS under the budget; the lines it queued and its error.
    fn call<F>(&mut self, label: &str, f: F) -> CallResult
    where
        F: for<'js> FnOnce(&Ctx<'js>) -> rquickjs::Result<()>,
    {
        self.interrupted.set(false);
        self.deadline.set(Some(Instant::now() + self.budget));
        let interrupted = self.interrupted.clone();
        let budget = self.budget;
        let error = self.ctx.with(|ctx| match f(&ctx) {
            Ok(()) => None,
            Err(rquickjs::Error::Exception) => {
                let caught = ctx.catch();
                Some(describe(label, &caught, interrupted.get().then_some(budget)))
            }
            Err(e) => Some(ScriptError {
                bar: format!("{label}: {e}"),
                full: format!("{label}: {e}"),
            }),
        });
        self.deadline.set(None);
        // A pending job (a promise reaction) would run with no budget later: run them now, under it.
        self.run_jobs();
        let mut error = error;
        // Promises still rejected with no handler: reported like any other script error (once: drained).
        let rejected = std::mem::take(&mut *self.rejected.borrow_mut());
        for (_, message, stack) in rejected {
            let e = build_error(&self.fallback, &format!("unhandled rejection: {message}"), &stack);
            error = Some(match error {
                None => e,
                Some(mut first) => {
                    first.full = format!("{}\n{}", first.full, e.full);
                    first
                }
            });
        }
        if error.is_some() {
            // An out-of-memory error leaves garbage (cycles) behind: collect it so the next call has room.
            self.rt.run_gc();
        }
        CallResult {
            lines: std::mem::take(&mut self.reg.borrow_mut().queue),
            error,
        }
    }

    /// Runs the promise jobs a call left (`Promise.resolve().then(...)`), each under the budget.
    fn run_jobs(&mut self) {
        while self.rt.is_job_pending() {
            self.deadline.set(Some(Instant::now() + self.budget));
            let r = self.rt.execute_pending_job();
            self.deadline.set(None);
            if r.is_err() {
                break;
            }
        }
    }
}

/// `web.map(mode, keys, rhs)`: checks everything at map time (an error throws, so it shows as
/// `init.js:LINE: web.map: ...`) and stores the mapping, replacing one with the same keys.
fn web_map<'js>(ctx: &Ctx<'js>, reg: &Rc<RefCell<Registry>>, mode: String, keys: String, rhs: Value<'js>) -> rquickjs::Result<()> {
    if mode != "n" {
        return Err(Exception::throw_type(
            ctx,
            &format!("web.map: unknown mode {mode:?} (known: \"n\")"),
        ));
    }
    let parsed = parse_map_keys(&keys).map_err(|m| Exception::throw_type(ctx, &m))?;
    let rhs = if let Some(f) = rhs.as_function() {
        MapRhs::Func(Persistent::save(ctx, f.clone()))
    } else if let Some(s) = rhs.as_string() {
        let s = s.to_string()?;
        MapRhs::Line(parse_map_rhs(&s).map_err(|m| Exception::throw_type(ctx, &m))?)
    } else {
        return Err(Exception::throw_type(
            ctx,
            &format!("web.map: rhs must be a ':' line or a function, not {}", rhs.type_name()),
        ));
    };
    let mut r = reg.borrow_mut();
    let mapping = Mapping {
        notation: keys,
        keys: parsed,
        rhs,
    };
    match r.maps.iter_mut().find(|m| m.keys == mapping.keys) {
        Some(old) => *old = mapping,
        None => r.maps.push(mapping),
    }
    Ok(())
}

/// `web` and its functions on the global object.
fn install_web<'js>(ctx: &Ctx<'js>, reg: &Rc<RefCell<Registry>>) -> rquickjs::Result<()> {
    let web = Object::new(ctx.clone())?;
    let r = reg.clone();
    web.set(
        "cmd",
        Function::new(ctx.clone(), move |line: String| {
            let line = line.trim();
            let line = line.strip_prefix(':').unwrap_or(line).to_string();
            r.borrow_mut().queue.push(line);
        })?,
    )?;
    let r = reg.clone();
    web.set(
        "command",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, name: String, f: Function<'js>| {
            if !valid_command_name(&name) {
                return Err(Exception::throw_type(
                    &ctx,
                    &format!("web.command: a command name starts with an uppercase letter and has no spaces: {name:?}"),
                ));
            }
            let f = Persistent::save(&ctx, f);
            r.borrow_mut().commands.insert(name, f);
            Ok(())
        })?,
    )?;
    let r = reg.clone();
    web.set(
        "map",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, mode: String, keys: String, rhs: Value<'js>| {
            web_map(&ctx, &r, mode, keys, rhs)
        })?,
    )?;
    let r = reg.clone();
    web.set(
        "on",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, event: String, f: Function<'js>| {
            if event != "ready" {
                return Err(Exception::throw_type(
                    &ctx,
                    &format!("web.on: unknown event {event:?} (known: \"ready\")"),
                ));
            }
            let f = Persistent::save(&ctx, f);
            r.borrow_mut().ready.push(f);
            Ok(())
        })?,
    )?;
    // Nothing else: `web` itself is frozen so a script cannot mistake an addition for an API.
    let freeze: Function = ctx.globals().get::<_, Object>("Object")?.get("freeze")?;
    freeze.call::<_, Value>((web.clone(),))?;
    ctx.globals().set("web", web)?;
    Ok(())
}

/// The bar's `FILE:LINE: message` and the log's full text for a caught exception.
fn describe(fallback: &str, v: &Value, interrupted: Option<Duration>) -> ScriptError {
    let (message, stack) = message_stack(v);
    let message = match interrupted {
        Some(b) => format!("interrupted: ran longer than SCRIPT_BUDGET_MS ({} ms)", b.as_millis()),
        None => message,
    };
    build_error(fallback, &message, &stack)
}

/// A thrown or rejected value's message and stack text.
fn message_stack(v: &Value) -> (String, String) {
    match v.as_object().and_then(|o| Exception::from_object(o.clone())) {
        Some(e) => (e.message().unwrap_or_default(), e.stack().unwrap_or_default()),
        None => (value_text(v), String::new()),
    }
}

/// A promise's identity (its heap address) for matching "rejected" with "handled later".
fn promise_id(p: &Value) -> usize {
    // SAFETY: reading the pointer member of a live object value; it is only compared, never followed.
    unsafe { p.as_raw().u.ptr as usize }
}

fn build_error(fallback: &str, message: &str, stack: &str) -> ScriptError {
    let at = stack_location(stack).unwrap_or_else(|| fallback.to_string());
    let bar = format!("{at}: {message}");
    let full = if stack.trim().is_empty() {
        bar.clone()
    } else {
        format!("{bar}\n{}", stack.trim_end())
    };
    ScriptError { bar, full }
}

/// A thrown non-Error value as text (`throw "x"`).
fn value_text(v: &Value) -> String {
    if let Some(s) = v.as_string() {
        return s.to_string().unwrap_or_default();
    }
    if let Some(n) = v.as_number() {
        return n.to_string();
    }
    if let Some(b) = v.as_bool() {
        return b.to_string();
    }
    format!("uncaught {}", v.type_name())
}

/// The first `FILE:LINE` in a QuickJS stack (`    at f (init.js:3:5)`, `    at init.js:3:5`);
/// frames without one (`(native)`) are skipped.
pub fn stack_location(stack: &str) -> Option<String> {
    for line in stack.lines() {
        let Some(rest) = line.trim_start().strip_prefix("at ") else {
            continue;
        };
        let loc = match (rest.rfind('('), rest.ends_with(')')) {
            (Some(i), true) => &rest[i + 1..rest.len() - 1],
            _ => rest,
        };
        let mut parts = loc.rsplitn(3, ':');
        let (Some(a), Some(b)) = (parts.next(), parts.next()) else {
            continue;
        };
        let digits = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
        match parts.next() {
            Some(file) if digits(a) && digits(b) && !file.is_empty() => return Some(format!("{file}:{b}")),
            None if digits(a) && !b.is_empty() => return Some(format!("{b}:{a}")),
            _ => {}
        }
    }
    None
}

// ---- the shell's side (`Keys`) ----

impl Keys {
    /// At start: runs each file that exists, in order (a missing file is not an error). An error
    /// shows in the bar and the log; the next file still runs.
    pub fn load_scripts(&mut self, files: &[ScriptFile]) {
        self.load_scripts_with(files, Duration::from_millis(SCRIPT_BUDGET_MS))
    }

    pub fn load_scripts_with(&mut self, files: &[ScriptFile], budget: Duration) {
        for f in files {
            let source = match std::fs::read_to_string(&f.path) {
                Ok(s) => s,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    self.script_error(ScriptError {
                        bar: format!("{}: {e}", f.label),
                        full: format!("{}: {e}", f.path.display()),
                    });
                    continue;
                }
            };
            self.load_script_source(&f.label, &source, budget);
        }
    }

    /// Runs one script's text (`load_scripts`, and tests).
    pub fn load_script_source(&mut self, label: &str, source: &str, budget: Duration) {
        if self.scripts.is_none() {
            match Scripts::new(budget) {
                Ok(s) => self.scripts = Some(s),
                Err(m) => {
                    self.script_error(ScriptError { bar: m.clone(), full: m });
                    return;
                }
            }
        }
        let start = Instant::now();
        let Some(s) = self.scripts.as_mut() else { return };
        let r = s.eval_file(label, source);
        s.held.extend(r.lines);
        self.effects
            .push(Effect::Log(format!("{label}: ran in {} ms", start.elapsed().as_millis())));
        if let Some(e) = r.error {
            self.script_error(e);
        }
    }

    /// `ready` (SCRIPTING.md): the host is connected and the first buffer exists. Runs the lines the
    /// files queued, then each handler (an error in one does not stop the next). Once per start.
    pub fn script_ready(&mut self) {
        let Some(s) = self.scripts.as_mut() else { return };
        let (held, handlers) = s.take_ready();
        if !handlers.is_empty() || !held.is_empty() {
            self.effects
                .push(Effect::Log(format!("script: ready ({} handlers)", handlers.len())));
        }
        self.run_script_lines(held);
        for f in handlers {
            let Some(s) = self.scripts.as_mut() else { return };
            let r = s.call_ready(f);
            self.finish_script_call(r);
        }
    }

    /// The user commands (completion offers them with the built-ins).
    pub fn user_command_names(&self) -> Vec<String> {
        self.scripts.as_ref().map(Scripts::command_names).unwrap_or_default()
    }

    /// `:Name args` for a user command; false when no script defined `name`.
    pub(crate) fn user_command(&mut self, name: &str, args: &[&str]) -> bool {
        let Some(s) = self.scripts.as_mut() else { return false };
        if !s.has_command(name) {
            return false;
        }
        if s.overflow {
            return true;
        }
        if s.depth >= SCRIPT_DEPTH_MAX {
            s.overflow = true;
            self.script_error(ScriptError {
                bar: format!("web.cmd: :{name}: user commands nested deeper than {SCRIPT_DEPTH_MAX}; stopped"),
                full: format!(
                    "web.cmd: :{name}: user commands nested deeper than SCRIPT_DEPTH_MAX ({SCRIPT_DEPTH_MAX}); the chain was stopped"
                ),
            });
            return true;
        }
        s.depth += 1;
        let r = s.call_command(name, args);
        self.finish_script_call(r);
        if let Some(s) = self.scripts.as_mut() {
            s.depth -= 1;
            if s.depth == 0 {
                s.overflow = false;
            }
        }
        true
    }

    /// After a call into JS: its error, then the lines it queued.
    pub(crate) fn finish_script_call(&mut self, r: CallResult) {
        if let Some(e) = r.error {
            self.script_error(e);
        }
        self.run_script_lines(r.lines);
    }

    /// `web.cmd`'s lines, each as typed after `:` (a chain past the depth limit drops the rest).
    fn run_script_lines(&mut self, lines: Vec<String>) {
        for line in lines {
            if self.scripts.as_ref().is_some_and(|s| s.overflow) {
                break;
            }
            self.effects.push(Effect::Log(format!("script: :{line}")));
            self.run_command_line(&line);
        }
    }

    fn script_error(&mut self, e: ScriptError) {
        self.effects.push(Effect::Log(format!("script error: {}", e.full)));
        self.message = e.bar;
    }
}
