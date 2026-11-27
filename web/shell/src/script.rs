//! C-SCRIPT: the user's init.js (SCRIPTING.md; DECISIONS.md 2026-11-24 "User scripts").
//!
//! QuickJS (rquickjs) with no `std`/`os` modules and no timers. One file runs per start, the profile's
//! `init.js`, evaluated as an ES module (always strict). C-SCRIPT-MODULES: it imports other modules
//! through our own resolver and loader (`ModuleDirs`, SCRIPTING.md "Modules"), confined to
//! `$XDG_CONFIG_HOME/web/`. The only thing a script sees is the global `web`: `web.cmd`,
//! `web.command`, `web.map` (C-SCRIPT-MAP), `web.on("ready")`. C-SCRIPT-URL: the built-in modules
//! `web:api` (the same `web`) and `web:url` (`URL`) are imported, never globals.
//!
//! Re-entrancy: no JS function ever holds a borrow of the shell. `web.cmd(line)` only appends the
//! line to a queue (a `RefCell` borrowed for the push and nothing else); after each call into JS
//! returns, `Keys` takes the lines that call queued and runs them in order, one `:` line at a time.
//! A line naming a user command calls into JS again from there (the previous call has returned), so
//! the order is the same as if `web.cmd` had run the line at once: `cmd("A"); cmd("B")` where `:A`
//! does `cmd("C")` runs A, C, B. The nesting of user commands through `web.cmd` is counted
//! (`SCRIPT_DEPTH_MAX`); past it the whole chain stops with an error.
//!
//! `init_files` gives at most one file (C-SCRIPT-MODULES: no global script any more); `:source` runs
//! it again in a new runtime, so edited modules are read again.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use rquickjs::context::EvalOptions;
use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::module::{Declarations, Exports, ModuleDef};
use rquickjs::promise::PromiseState;
use rquickjs::{Context, Ctx, Exception, Function, JsLifetime, Module, Object, Persistent, Promise, Runtime, Value};

use crate::buffers::BufferInfo;
use crate::config::{SCRIPT_BUDGET_MS, SCRIPT_CHAIN_MAX, SCRIPT_DEPTH_MAX, SCRIPT_MEMORY_MAX, SCRIPT_MODULE_MAX};
use crate::keys::{parse_map_keys, parse_map_rhs, resolve_mapping, Effect, Keys, MapKey, MapLine, MapStep};
use crate::picker::PickAction;

/// One script to load: where it is, the name the bar gives it (`init.js`) and the config directory
/// (`$XDG_CONFIG_HOME/web`) its imports are confined to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptFile {
    pub path: PathBuf,
    pub label: String,
    pub root: PathBuf,
}

/// The script web loads at start (C-SCRIPT-MODULES: one file, nothing else is read):
/// `$XDG_CONFIG_HOME/web/profiles/PROFILE/init.js` (default `~/.config/web/...`). `profile` is
/// `default` without `--profile` (args.rs). Errors in it are labelled `init.js`. Empty when there is
/// no config directory at all.
pub fn init_files(env: &dyn Fn(&str) -> Option<String>, profile: &str) -> Vec<ScriptFile> {
    let set = |k: &str| env(k).filter(|v| !v.is_empty());
    let config = match (set("XDG_CONFIG_HOME"), set("HOME")) {
        (Some(x), _) if Path::new(&x).is_absolute() => PathBuf::from(x),
        (_, Some(h)) => Path::new(&h).join(".config"),
        _ => return Vec::new(),
    };
    let root = config.join("web");
    vec![ScriptFile {
        path: root.join("profiles").join(profile).join("init.js"),
        label: "init.js".to_string(),
        root,
    }]
}

/// C-SCRIPT-MODULES: where one runtime's imports may come from (SCRIPTING.md "Modules").
#[derive(Debug, Clone, Default)]
pub struct ModuleDirs {
    /// `$XDG_CONFIG_HOME/web` as written and with its symlinks resolved. None: every import is
    /// refused (a script given as text, in tests).
    root: Option<(PathBuf, PathBuf)>,
    /// The profile's folder: what a bare specifier (`"common.js"`) is relative to.
    profile: PathBuf,
}

impl ModuleDirs {
    /// For `file`: confined to its `root`, bare names from the file's own folder. A root that does
    /// not exist (so has no real path) allows nothing.
    pub fn of(file: &ScriptFile) -> ModuleDirs {
        ModuleDirs {
            root: file.root.canonicalize().ok().map(|real| (file.root.clone(), real)),
            profile: file.path.parent().map(Path::to_path_buf).unwrap_or_default(),
        }
    }

    /// `specifier` imported from the module named `base`: the module's name (its real path), or why
    /// it is refused (the rule and the path). Nothing is read; `read_script` checks the open file again.
    pub fn resolve(&self, base: &str, specifier: &str) -> Result<String, String> {
        let Some((root, real_root)) = &self.root else {
            return Err("no config directory to import from".to_string());
        };
        let p = Path::new(specifier);
        let joined = if specifier.starts_with("./") || specifier.starts_with("../") {
            Path::new(base).parent().unwrap_or(Path::new("")).join(p)
        } else if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.profile.join(p)
        };
        let path = normalize(&joined);
        let shown = path.display();
        if !path.is_absolute() {
            return Err(format!("not an absolute path: {shown}"));
        }
        if path.extension().is_none_or(|e| e != "js") {
            return Err(format!("not a .js file: {shown}"));
        }
        let outside = || format!("outside {}/: {shown}", root.display());
        // By text first, so `../` past the root is refused whether or not the file exists.
        if !path.starts_with(root) && !path.starts_with(real_root) {
            return Err(outside());
        }
        // Then on the real path: a symlink inside the root must not lead out of it.
        let real = match path.canonicalize() {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(format!("no such file: {shown}")),
            Err(e) => return Err(format!("{shown}: {e}")),
        };
        if !real.starts_with(real_root) {
            return Err(format!("{} (a symlink to {})", outside(), real.display()));
        }
        if real.extension().is_none_or(|e| e != "js") {
            return Err(format!("not a .js file: {shown} (a symlink to {})", real.display()));
        }
        let meta = std::fs::metadata(&real).map_err(|e| format!("{shown}: {e}"))?;
        check_script_meta(&meta).map_err(|why| format!("{why}: {shown}"))?;
        real.into_os_string()
            .into_string()
            .map_err(|p| format!("not a UTF-8 path: {}", Path::new(&p).display()))
    }
}

/// `p` with `.` and `..` taken out by text (no file system access; `/..` stays `/`).
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c.as_os_str()),
        }
    }
    out
}

/// A script file must be a regular file (no FIFO, device, directory) of at most `SCRIPT_MODULE_MAX`.
fn check_script_meta(meta: &std::fs::Metadata) -> Result<(), String> {
    if !meta.is_file() {
        return Err("not a regular file".to_string());
    }
    if meta.len() > SCRIPT_MODULE_MAX {
        return Err(format!("larger than SCRIPT_MODULE_MAX ({SCRIPT_MODULE_MAX} bytes)"));
    }
    Ok(())
}

/// Reads a script file (init.js or a module it imports): opened without blocking (a FIFO swapped in
/// after the check cannot hang the shell), checked on the open file, read up to `SCRIPT_MODULE_MAX`.
/// A missing file keeps `ErrorKind::NotFound`; a refusal names the rule.
pub fn read_script(path: &Path) -> std::io::Result<String> {
    let file = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(path)?;
    check_script_meta(&file.metadata()?).map_err(std::io::Error::other)?;
    let mut bytes = Vec::new();
    file.take(SCRIPT_MODULE_MAX + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > SCRIPT_MODULE_MAX {
        return Err(std::io::Error::other(format!(
            "larger than SCRIPT_MODULE_MAX ({SCRIPT_MODULE_MAX} bytes)"
        )));
    }
    String::from_utf8(bytes).map_err(|_| std::io::Error::other("not UTF-8"))
}

/// rquickjs's side of `ModuleDirs` (our own, not its FileResolver/ScriptLoader: no search path, no
/// extension guessing). A refusal throws `import "SPEC": RULE: PATH`.
struct WebModules(Rc<ModuleDirs>);

impl Resolver for WebModules {
    fn resolve<'js>(&mut self, ctx: &Ctx<'js>, base: &str, name: &str, _: Option<ImportAttributes<'js>>) -> rquickjs::Result<String> {
        // C-SCRIPT-URL: `web:` names are checked before any file rule (they work with no config
        // directory, and no file can shadow one). The module's name is the specifier itself, so
        // QuickJS loads it once per runtime and every importer gets the same objects.
        let resolved = if name.starts_with(BUILTIN_PREFIX) {
            match BUILTIN_MODULES.contains(&name) {
                true => Ok(name.to_string()),
                false => Err("no such built-in module".to_string()),
            }
        } else {
            self.0.resolve(base, name)
        };
        resolved.map_err(|why| Exception::throw_message(ctx, &format!("import {name:?}: {why}")))
    }
}

impl Loader for WebModules {
    fn load<'js>(&mut self, ctx: &Ctx<'js>, name: &str, _: Option<ImportAttributes<'js>>) -> rquickjs::Result<Module<'js>> {
        // A built-in is made from what is compiled in: no file is read. (A file's name is its absolute
        // path, so it never starts with `web:`.)
        match name {
            "web:api" => return Module::declare_def::<WebApiModule, _>(ctx.clone(), name),
            "web:url" => return Module::declare_def::<WebUrlModule, _>(ctx.clone(), name),
            _ => {}
        }
        let source = read_script(Path::new(name)).map_err(|e| Exception::throw_message(ctx, &format!("import {name}: {e}")))?;
        Module::declare(ctx.clone(), name, source)
    }
}

/// C-SCRIPT-URL: the reserved prefix of the built-in modules (SCRIPTING.md "Modules").
const BUILTIN_PREFIX: &str = "web:";
/// Every built-in module; any other `web:` name is refused (names are case-sensitive).
const BUILTIN_MODULES: [&str; 2] = ["web:api", "web:url"];

/// The original `web` object, kept in the context's userdata by `install_web` so `web:api` gives it
/// back whatever a script did to `globalThis.web`. Removed in `Scripts::drop`.
struct WebObject<'js>(Object<'js>);

// SAFETY: the only `'js` value is the `Object<'js>`; `Changed` swaps exactly that lifetime.
unsafe impl<'js> JsLifetime<'js> for WebObject<'js> {
    type Changed<'to> = WebObject<'to>;
}

/// The `web` object `install_web` made.
fn web_object<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
    match ctx.userdata::<WebObject>() {
        Some(w) => Ok(w.0.clone()),
        None => Err(Exception::throw_internal(ctx, "web:api: no web object")),
    }
}

/// `web:api`: `web` as the default export and one named export per key of `web` (read from the
/// object itself, so a key added to `web` later is exported too).
struct WebApiModule;

impl ModuleDef for WebApiModule {
    fn declare<'js>(decl: &Declarations<'js>) -> rquickjs::Result<()> {
        let web = web_object(decl.module().ctx())?;
        decl.declare("default")?;
        for key in web.keys::<String>() {
            decl.declare(key?)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> rquickjs::Result<()> {
        let web = web_object(ctx)?;
        for key in web.keys::<String>() {
            let key = key?;
            exports.export(key.as_str(), web.get::<_, Value>(key.as_str())?)?;
        }
        exports.export("default", web)?;
        Ok(())
    }
}

/// `web:url`: the named export `URL` (no default export).
struct WebUrlModule;

impl ModuleDef for WebUrlModule {
    fn declare<'js>(decl: &Declarations<'js>) -> rquickjs::Result<()> {
        decl.declare("URL")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> rquickjs::Result<()> {
        let parse = Function::new(
            ctx.clone(),
            |ctx: Ctx<'js>, input: String, base: Option<String>| -> rquickjs::Result<Value<'js>> {
                let Some(parts) = url_parts(&input, base.as_deref()) else {
                    return Ok(Value::new_null(ctx));
                };
                let o = Object::new(ctx.clone())?;
                for (k, v) in parts {
                    o.set(k, v)?;
                }
                Ok(o.into_value())
            },
        )?;
        // Compiled under the name `web:url`: its stack frames are the built-in's (`stack_location`
        // skips them), and the factory is an expression, so nothing is left on the global object.
        let mut opts = EvalOptions::default();
        opts.strict = true;
        opts.filename = Some("web:url".to_string());
        let factory: Function = ctx.eval_with_options(URL_JS, opts)?;
        let class: Value = factory.call((parse,))?;
        exports.export("URL", class)?;
        Ok(())
    }
}

/// The `URL` class around the native parser (`url_parts`): parsed once in the constructor, enumerable
/// getters only, prototype and constructor frozen.
const URL_JS: &str = r#"(parse) => {
  const arg = (v) => (v === undefined ? undefined : String(v));
  class URL {
    #p;
    constructor(input, base) {
      const p = parse(String(input), arg(base));
      if (p === null) throw new TypeError("Invalid URL: " + String(input));
      this.#p = p;
    }
    get href() { return this.#p.href; }
    get protocol() { return this.#p.protocol; }
    get username() { return this.#p.username; }
    get password() { return this.#p.password; }
    get host() { return this.#p.host; }
    get hostname() { return this.#p.hostname; }
    get port() { return this.#p.port; }
    get origin() { return this.#p.origin; }
    get pathname() { return this.#p.pathname; }
    get search() { return this.#p.search; }
    get hash() { return this.#p.hash; }
    toString() { return this.#p.href; }
    toJSON() { return this.#p.href; }
    static canParse(input, base) {
      try { return parse(String(input), arg(base)) !== null; } catch { return false; }
    }
    static parse(input, base) {
      try { return new URL(input, base); } catch { return null; }
    }
  }
  for (const k of ["href", "protocol", "username", "password", "host", "hostname", "port", "origin", "pathname", "search", "hash"]) {
    const d = Object.getOwnPropertyDescriptor(URL.prototype, k);
    d.enumerable = true;
    Object.defineProperty(URL.prototype, k, d);
  }
  Object.freeze(URL.prototype);
  Object.freeze(URL);
  return URL;
}"#;

/// C-SCRIPT-URL: `input` parsed (against `base` when given) by the `url` crate, the WHATWG parser the
/// shell uses, as the `URL` getters' values (SCRIPTING.md "web:url"). None when either does not parse.
pub fn url_parts(input: &str, base: Option<&str>) -> Option<Vec<(&'static str, String)>> {
    let base = match base {
        Some(b) => Some(url::Url::parse(b).ok()?),
        None => None,
    };
    let u = url::Url::options().base_url(base.as_ref()).parse(input).ok()?;
    let hostname = u.host_str().unwrap_or("").to_string();
    let port = u.port().map(|p| p.to_string()).unwrap_or_default();
    let host = if port.is_empty() {
        hostname.clone()
    } else {
        format!("{hostname}:{port}")
    };
    // WHATWG: an empty query or fragment gives "" (`https://a.test/?` has no search).
    let prefixed = |sign: &str, v: Option<&str>| v.filter(|v| !v.is_empty()).map(|v| format!("{sign}{v}")).unwrap_or_default();
    Some(vec![
        ("href", u.as_str().to_string()),
        ("protocol", format!("{}:", u.scheme())),
        ("username", u.username().to_string()),
        ("password", u.password().unwrap_or("").to_string()),
        ("hostname", hostname),
        ("port", port),
        ("host", host),
        ("origin", u.origin().ascii_serialization()),
        ("pathname", u.path().to_string()),
        ("search", prefixed("?", u.query())),
        ("hash", prefixed("#", u.fragment())),
    ])
}

/// An error out of a script: what the status bar shows (`init.js:LINE: message`) and the full
/// text for the log (with the stack).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptError {
    pub bar: String,
    pub full: String,
}

/// C-CMDRESULT: one `web.cmd` line and the function that settles its promise (SCRIPTING.md `web.cmd`).
pub struct ScriptLine {
    pub line: String,
    pub resolve: Option<Persistent<Function<'static>>>,
}

impl std::fmt::Debug for ScriptLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ScriptLine({:?})", self.line)
    }
}

/// What one call into JS left behind: the `web.cmd` lines it queued, in order, and its error.
#[derive(Debug, Default)]
pub struct CallResult {
    pub lines: Vec<ScriptLine>,
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

/// C-PICKDEL-MAP: the selected row as a "p" mapping's function gets it.
pub struct PickRow {
    pub source: String,
    pub label: String,
    pub detail: String,
    pub action: PickAction,
}

/// One `web.map("n", keys, rhs)`: the keys as written (for the log) and parsed.
struct Mapping {
    notation: String,
    keys: Vec<MapKey>,
    rhs: MapRhs,
}

/// C-PICKDEL: whether Ctrl-d in each picker asks before deleting (default: yes).
struct DeleteConfirm {
    history: bool,
    bookmarks: bool,
    buffers: bool,
}

impl Default for DeleteConfirm {
    fn default() -> Self {
        DeleteConfirm {
            history: true,
            bookmarks: true,
            buffers: true,
        }
    }
}

/// What the scripts registered (shared with the `web` functions; borrowed only for a push or a lookup).
#[derive(Default)]
struct Registry {
    /// C-PICKDEL: `web.history/bookmarks/buffers.deleteConfirmation`.
    confirm: DeleteConfirm,
    /// C-FOCUSEXISTING: `web.buffers.focusExisting` (default false).
    focus_existing: bool,
    /// C-MAPTIMEOUT: `web.settings.mapTimeout` in ms (None = `MAP_TIMEOUT_MS`).
    map_timeout: Option<u64>,
    queue: Vec<ScriptLine>,
    commands: BTreeMap<String, Persistent<Function<'static>>>,
    ready: Vec<Persistent<Function<'static>>>,
    /// NORMAL-mode mappings, in the order made (mapping the same keys again replaces in place).
    maps: Vec<Mapping>,
    /// C-PICKDEL-MAP: PICK mappings (`web.map("p", "<C-x>", fn)`), same replace-in-place rule.
    pick_maps: Vec<Mapping>,
    /// C-SCRIPT-BUFLIST: the buffers as of the start of the current call (`Keys::script_sync`);
    /// what `web.buffers.list()` returns. Empty until `ready`.
    buffers: Vec<BufferInfo>,
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
    /// The label errors without a script frame get (`init.js`).
    fallback: String,
    /// The init file's module name (its path); a stack frame in it is shown as `fallback`.
    init_name: String,
    /// C-SCRIPT-MODULES: the init module's evaluation promise while a top-level `await` keeps it
    /// pending; whichever later call settles it reports its rejection as the file's error.
    module_promise: Rc<RefCell<Option<Persistent<Promise<'static>>>>>,
    /// The promise of the module evaluated in this call: its rejection is the call's error, so the
    /// rejection tracker must not report it again.
    module_seen: Rc<Cell<Option<usize>>>,
    /// Set while the init module is compiled, linked and its imports loaded: an error then is the
    /// init file's (`init.js: /path/to/module.js:3: ...`).
    loading: Rc<Cell<bool>>,
    /// Files evaluated so far.
    files: usize,
    /// `web.cmd` lines queued by the file itself (and its modules): run when `ready` fires (before the
    /// handlers), since before that there is no host to send commands to.
    held: Vec<ScriptLine>,
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
        self.module_promise.borrow_mut().take();
        self.ctx.with(|ctx| {
            let _ = ctx.remove_userdata::<WebObject>();
        });
        let mut r = self.reg.borrow_mut();
        // Lines still waiting for `ready` hold promise resolvers: they go with the runtime (`:source`
        // drops a pending promise without a word).
        self.held.clear();
        r.queue.clear();
        r.commands.clear();
        r.ready.clear();
        r.maps.clear();
        r.pick_maps.clear();
    }
}

/// `web.command`'s name rule: an uppercase first letter (so a user command never shadows a
/// built-in one) and no whitespace (it must be typeable as one word).
pub fn valid_command_name(name: &str) -> bool {
    name.chars().next().is_some_and(char::is_uppercase) && !name.chars().any(char::is_whitespace)
}

impl Scripts {
    /// A runtime with `web` installed and nothing else beyond the language itself; its imports
    /// come from `dirs` only.
    pub fn new(budget: Duration, dirs: ModuleDirs) -> Result<Scripts, String> {
        let rt = Runtime::new().map_err(|e| format!("script runtime: {e}"))?;
        let dirs = Rc::new(dirs);
        rt.set_loader(WebModules(dirs.clone()), WebModules(dirs));
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
            init_name: "init.js".to_string(),
            module_promise: Rc::default(),
            module_seen: Rc::default(),
            loading: Rc::default(),
            files: 0,
            held: Vec::new(),
            ready_fired: false,
            depth: 0,
            overflow: false,
            ctx,
            rt,
        })
    }

    /// Runs the init file as an ES module named `name` (its path: relative imports resolve against
    /// it), errors labelled `label`. Its imports are loaded and everything is evaluated in this one
    /// budgeted call; the evaluation promise is driven until it settles (a top-level throw rejects it:
    /// that is this call's error, reported once). A top-level `await` still pending when no job is left
    /// waits for a later call (a `ready` handler, a command) to settle it; the start is finished, not
    /// failed. What ran before an error stays.
    pub fn eval_module(&mut self, name: &str, label: &str, source: &str) -> CallResult {
        if self.files == 0 {
            self.fallback = label.to_string();
            self.init_name = name.to_string();
        }
        self.files += 1;
        let (name, source) = (name.to_string(), source.to_string());
        let (pending, seen, loading) = (self.module_promise.clone(), self.module_seen.clone(), self.loading.clone());
        self.call(label, move |ctx| {
            loading.set(true);
            let (_, promise) = Module::declare(ctx.clone(), name, source)?.eval()?;
            loading.set(false);
            seen.set(Some(promise_id(promise.as_value())));
            match promise.finish::<Value>() {
                Err(rquickjs::Error::WouldBlock) => {
                    *pending.borrow_mut() = Some(Persistent::save(ctx, promise));
                    Ok(())
                }
                r => r.map(|_| ()),
            }
        })
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

    /// C-PICKDEL: whether Ctrl-d in picker `source` asks first (anything but history, bookmarks,
    /// buffers: true).
    pub fn delete_confirmation(&self, source: &str) -> bool {
        let c = &self.reg.borrow().confirm;
        match source {
            "history" => c.history,
            "bookmarks" => c.bookmarks,
            "buffers" => c.buffers,
            _ => true,
        }
    }

    /// C-FOCUSEXISTING: `web.buffers.focusExisting`: whether a plain click on a `target=_blank` link
    /// to a URL a buffer already shows switches to that buffer instead of opening a new one.
    pub fn focus_existing(&self) -> bool {
        self.reg.borrow().focus_existing
    }

    /// C-MAPTIMEOUT: `web.settings.mapTimeout`: how long the wait for the rest of a mapping lasts, in ms.
    pub fn map_timeout(&self) -> u64 {
        self.reg.borrow().map_timeout.unwrap_or(crate::config::MAP_TIMEOUT_MS)
    }

    /// C-SCRIPT-BUFLIST: the snapshot the next call's `web.buffers.list()` reads.
    pub fn set_buffers(&mut self, infos: Vec<BufferInfo>) {
        self.reg.borrow_mut().buffers = infos;
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

    /// C-PICKDEL-MAP: the function of the "p" mapping for this press, with its keys as written.
    pub fn pick_map(&self, key: &MapKey) -> Option<(String, Persistent<Function<'static>>)> {
        let r = self.reg.borrow();
        r.pick_maps.iter().find(|m| m.keys.as_slice() == [*key]).and_then(|m| match &m.rhs {
            MapRhs::Func(f) => Some((m.notation.clone(), f.clone())),
            MapRhs::Line(_) => None,
        })
    }

    /// C-PICKDEL-MAP: calls a "p" mapping's function with the selected row as a plain object.
    pub fn call_pick_map(&mut self, f: Persistent<Function<'static>>, row: PickRow) -> CallResult {
        let label = self.fallback.clone();
        self.call(&label, move |ctx| {
            let o = Object::new(ctx.clone())?;
            o.set("source", row.source)?;
            o.set("label", row.label)?;
            o.set("detail", row.detail)?;
            match row.action {
                PickAction::Open(u) => o.set("url", u)?,
                PickAction::Buffer(n) => o.set("buffer", n)?,
                PickAction::Popup(n) => o.set("popup", n)?,
                PickAction::Notice(n) => o.set("notice", n as f64)?,
                PickAction::FileEntry { path, .. } | PickAction::FileHere(path) => o.set("path", path)?,
            }
            f.restore(ctx)?.call::<_, Value>((o,)).map(|_| ())
        })
    }

    /// C-CMDRESULT: settles a `web.cmd` promise with `{ ok, message }` in a budgeted call of its own
    /// (its `await` continuation and the promise jobs run here, under the same budget and error report
    /// as any call); the lines that continuation queued come back in the result.
    pub fn settle(&mut self, resolve: Persistent<Function<'static>>, ok: bool, message: String) -> CallResult {
        let label = self.fallback.clone();
        self.call(&label, move |ctx| {
            let o = Object::new(ctx.clone())?;
            o.set("ok", ok)?;
            o.set("message", message)?;
            resolve.restore(ctx)?.call::<_, Value>((o,)).map(|_| ())
        })
    }

    /// The `ready` handlers in order (the event fires once: later calls give nothing).
    pub fn take_ready(&mut self) -> (Vec<ScriptLine>, Vec<Persistent<Function<'static>>>) {
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
        let init = self.init_name.clone();
        let error = self.ctx.with(|ctx| match f(&ctx) {
            Ok(()) => None,
            Err(rquickjs::Error::Exception) => {
                let caught = ctx.catch();
                Some(describe((&init, label), &caught, interrupted.get().then_some(budget)))
            }
            Err(e) => Some(ScriptError {
                bar: format!("{label}: {e}"),
                full: format!("{label}: {e}"),
            }),
        });
        self.deadline.set(None);
        // An import that failed (not found, refused, a syntax error in the module) is the init file's
        // error, with the module's path in the message.
        let error = match error {
            Some(e) if self.loading.replace(false) && !e.bar.starts_with(&format!("{label}:")) => Some(ScriptError {
                bar: format!("{label}: {}", e.bar),
                full: format!("{label}: {}", e.full),
            }),
            e => e,
        };
        // A pending job (a promise reaction) would run with no budget later: run them now, under it.
        self.run_jobs();
        let mut error = error;
        let mut module_ids = Vec::from_iter(self.module_seen.take());
        // A top-level await that a callback (this call) let go on: its rejection is the file's error.
        if let Some((id, e)) = self.settled_module() {
            module_ids.push(id);
            error = join_errors(error, e);
        }
        // Promises still rejected with no handler: reported like any other script error (once: drained).
        let rejected = std::mem::take(&mut *self.rejected.borrow_mut());
        for (_, message, stack) in rejected.into_iter().filter(|(id, _, _)| !module_ids.contains(id)) {
            let e = build_error(
                (&self.init_name, &self.fallback),
                &format!("unhandled rejection: {message}"),
                &stack,
            );
            error = join_errors(error, Some(e));
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

    /// The init module left pending by a top-level `await`, once it has settled: its promise's
    /// identity and, when it was rejected, the error (labelled as the file's own errors are).
    fn settled_module(&mut self) -> Option<(usize, Option<ScriptError>)> {
        let p = self.module_promise.borrow().clone()?;
        let names = (self.init_name.clone(), self.fallback.clone());
        let interrupted = self.interrupted.get().then_some(self.budget);
        let settled = self.ctx.with(|ctx| {
            let p = p.restore(&ctx).ok()?;
            let id = promise_id(p.as_value());
            match p.state() {
                PromiseState::Pending => None,
                PromiseState::Resolved => Some((id, None)),
                PromiseState::Rejected => {
                    // `result` throws the reason; take it back off the context.
                    let _ = p.result::<Value>();
                    let reason = ctx.catch();
                    Some((id, Some(describe((&names.0, &names.1), &reason, interrupted))))
                }
            }
        });
        if settled.is_some() {
            self.module_promise.borrow_mut().take();
        }
        settled
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
    if mode != "n" && mode != "p" {
        return Err(Exception::throw_type(
            ctx,
            &format!("web.map: unknown mode {mode:?} (known: \"n\", \"p\")"),
        ));
    }
    let parsed = parse_map_keys(&keys).map_err(|m| Exception::throw_type(ctx, &m))?;
    if mode == "p" {
        if parsed.len() != 1 || !parsed[0].ctrl {
            return Err(Exception::throw_type(
                ctx,
                "web.map: a \"p\" mapping is one <C-x> key (letters type the filter)",
            ));
        }
        let Some(f) = rhs.as_function() else {
            return Err(Exception::throw_type(ctx, "web.map: a \"p\" mapping's rhs is a function"));
        };
        let mapping = Mapping {
            notation: keys,
            keys: parsed,
            rhs: MapRhs::Func(Persistent::save(ctx, f.clone())),
        };
        let mut r = reg.borrow_mut();
        match r.pick_maps.iter_mut().find(|m| m.keys == mapping.keys) {
            Some(old) => *old = mapping,
            None => r.pick_maps.push(mapping),
        }
        return Ok(());
    }
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
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, line: String| -> rquickjs::Result<Promise<'js>> {
            let line = line.trim();
            let line = line.strip_prefix(':').unwrap_or(line).to_string();
            // C-CMDRESULT: settled with `{ ok, message }` after the line ran; never rejected.
            let (promise, resolve, _reject) = ctx.promise()?;
            let resolve = Some(Persistent::save(&ctx, resolve));
            r.borrow_mut().queue.push(ScriptLine { line, resolve });
            Ok(promise)
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
    // C-PICKDEL: `web.history`, `web.bookmarks`, `web.buffers`, each with the accessor `deleteConfirmation`.
    let define: Function = ctx.globals().get::<_, Object>("Object")?.get("defineProperty")?;
    let freeze: Function = ctx.globals().get::<_, Object>("Object")?.get("freeze")?;
    for name in ["history", "bookmarks", "buffers"] {
        let obj = Object::new(ctx.clone())?;
        let r = reg.clone();
        let get = Function::new(ctx.clone(), move || {
            let c = &r.borrow().confirm;
            match name {
                "history" => c.history,
                "bookmarks" => c.bookmarks,
                _ => c.buffers,
            }
        })?;
        let r = reg.clone();
        let set = Function::new(ctx.clone(), move |ctx: Ctx<'js>, v: Value<'js>| {
            let Some(b) = v.as_bool() else {
                return Err(Exception::throw_type(
                    &ctx,
                    &format!("web.{name}.deleteConfirmation must be true or false"),
                ));
            };
            let c = &mut r.borrow_mut().confirm;
            match name {
                "history" => c.history = b,
                "bookmarks" => c.bookmarks = b,
                _ => c.buffers = b,
            }
            Ok(())
        })?;
        let desc = Object::new(ctx.clone())?;
        desc.set("get", get)?;
        desc.set("set", set)?;
        desc.set("enumerable", true)?;
        define.call::<_, Value>((obj.clone(), "deleteConfirmation", desc))?;
        if name == "buffers" {
            // C-FOCUSEXISTING: `web.buffers.focusExisting`, a boolean accessor like deleteConfirmation.
            let r = reg.clone();
            let get = Function::new(ctx.clone(), move || r.borrow().focus_existing)?;
            let r = reg.clone();
            let set = Function::new(ctx.clone(), move |ctx: Ctx<'js>, v: Value<'js>| {
                let Some(b) = v.as_bool() else {
                    return Err(Exception::throw_type(&ctx, "web.buffers.focusExisting must be true or false"));
                };
                r.borrow_mut().focus_existing = b;
                Ok(())
            })?;
            let desc = Object::new(ctx.clone())?;
            desc.set("get", get)?;
            desc.set("set", set)?;
            desc.set("enumerable", true)?;
            define.call::<_, Value>((obj.clone(), "focusExisting", desc))?;
            // C-SCRIPT-BUFLIST: a function, so it is clearly read when called. Fresh frozen objects.
            let r = reg.clone();
            let list = Function::new(ctx.clone(), move |ctx: Ctx<'js>| -> rquickjs::Result<rquickjs::Array<'js>> {
                let infos = r.borrow().buffers.clone();
                let freeze: Function = ctx.globals().get::<_, Object>("Object")?.get("freeze")?;
                let arr = rquickjs::Array::new(ctx.clone())?;
                for (i, b) in infos.into_iter().enumerate() {
                    let o = Object::new(ctx.clone())?;
                    o.set("number", b.number)?;
                    o.set("url", b.url)?;
                    o.set("title", b.title)?;
                    o.set("description", b.description)?;
                    o.set("current", b.current)?;
                    o.set("alternate", b.alternate)?;
                    o.set("loading", b.loading)?;
                    o.set("crashed", b.crashed)?;
                    o.set("unread", b.unread as u32)?;
                    freeze.call::<_, Value>((o.clone(),))?;
                    arr.set(i, o)?;
                }
                Ok(arr)
            })?;
            obj.set("list", list)?;
        }
        freeze.call::<_, Value>((obj.clone(),))?;
        web.set(name, obj)?;
    }
    // C-MAPTIMEOUT: `web.settings`, for settings that belong to no source.
    {
        let obj = Object::new(ctx.clone())?;
        let r = reg.clone();
        let get = Function::new(ctx.clone(), move || r.borrow().map_timeout.unwrap_or(crate::config::MAP_TIMEOUT_MS))?;
        let r = reg.clone();
        let set = Function::new(ctx.clone(), move |ctx: Ctx<'js>, v: Value<'js>| {
            let ok = v.as_number().filter(|n| n.fract() == 0.0 && (1.0..=10000.0).contains(n));
            let Some(n) = ok else {
                return Err(Exception::throw_type(
                    &ctx,
                    "web.settings.mapTimeout must be an integer from 1 to 10000 (milliseconds)",
                ));
            };
            r.borrow_mut().map_timeout = Some(n as u64);
            Ok(())
        })?;
        let desc = Object::new(ctx.clone())?;
        desc.set("get", get)?;
        desc.set("set", set)?;
        desc.set("enumerable", true)?;
        define.call::<_, Value>((obj.clone(), "mapTimeout", desc))?;
        freeze.call::<_, Value>((obj.clone(),))?;
        web.set("settings", obj)?;
    }
    // Nothing else: `web` itself is frozen so a script cannot mistake an addition for an API.
    freeze.call::<_, Value>((web.clone(),))?;
    // C-SCRIPT-URL: the same object is `web:api`'s default export, whatever happens to the global.
    if ctx.store_userdata(WebObject(web.clone())).is_err() {
        return Err(Exception::throw_internal(ctx, "web:api: cannot keep the web object"));
    }
    ctx.globals().set("web", web)?;
    Ok(())
}

/// `(init file's module name, its label)`: a stack frame in the init file is shown with the label
/// (`init.js:3`), a frame in an imported module with that module's path; no frame: the label.
type Names<'a> = (&'a str, &'a str);

/// Two errors of one call: the first is what the bar shows, the log gets both.
fn join_errors(first: Option<ScriptError>, second: Option<ScriptError>) -> Option<ScriptError> {
    match (first, second) {
        (Some(mut a), Some(b)) => {
            a.full = format!("{}\n{}", a.full, b.full);
            Some(a)
        }
        (a, b) => a.or(b),
    }
}

/// The bar's `FILE:LINE: message` and the log's full text for a caught exception.
fn describe(names: Names, v: &Value, interrupted: Option<Duration>) -> ScriptError {
    let (message, stack) = message_stack(v);
    let message = match interrupted {
        Some(b) => format!("interrupted: ran longer than SCRIPT_BUDGET_MS ({} ms)", b.as_millis()),
        None => message,
    };
    build_error(names, &message, &stack)
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

fn build_error((init, label): Names, message: &str, stack: &str) -> ScriptError {
    let at = match stack_location(stack) {
        Some(loc) => match loc.rsplit_once(':') {
            Some((file, line)) if file == init => format!("{label}:{line}"),
            _ => loc,
        },
        None => label.to_string(),
    };
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
/// frames without one (`(native)`) and frames in a built-in module (`web:url`) are skipped.
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
            // C-SCRIPT-URL: a frame in a built-in module (`web:url`) is skipped too, so an error
            // thrown inside one (`new URL("x")`) is shown at the user's line that called it.
            Some(file) if file.starts_with(BUILTIN_PREFIX) => {}
            Some(file) if digits(a) && digits(b) && !file.is_empty() => return Some(format!("{file}:{b}")),
            None if digits(a) && !b.is_empty() => return Some(format!("{b}:{a}")),
            _ => {}
        }
    }
    None
}

// ---- the shell's side (`Keys`) ----

impl Keys {
    /// At start: runs the init file if it exists (`init_files`: at most one; a missing file is not
    /// an error). An error shows in the bar and the log.
    pub fn load_scripts(&mut self, files: &[ScriptFile]) -> (usize, usize) {
        self.load_scripts_with(files, Duration::from_millis(SCRIPT_BUDGET_MS))
    }

    /// Returns how many files were read and run, and how many of those (or their reading) failed.
    /// Remembers the files and the budget for `:source`.
    pub fn load_scripts_with(&mut self, files: &[ScriptFile], budget: Duration) -> (usize, usize) {
        self.script_files = files.to_vec();
        self.script_budget = budget;
        let (mut ran, mut failed) = (0, 0);
        for f in files {
            // The same checks as an import's (a regular file under SCRIPT_MODULE_MAX), so a FIFO or
            // a huge file cannot hold up the start.
            let source = match read_script(&f.path) {
                Ok(s) => s,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    self.script_error(ScriptError {
                        bar: format!("{}: {e}", f.label),
                        full: format!("{}: {e}", f.path.display()),
                    });
                    failed += 1;
                    continue;
                }
            };
            ran += 1;
            let name = f.path.display().to_string();
            if !self.run_init(&name, &f.label, ModuleDirs::of(f), &source, budget) {
                failed += 1;
            }
        }
        (ran, failed)
    }

    /// Runs a script's text as the init module named `label`, with no file behind it (tests): every
    /// import is refused. False when it failed.
    #[cfg(test)]
    pub fn load_script_source(&mut self, label: &str, source: &str, budget: Duration) -> bool {
        self.run_init(label, label, ModuleDirs::default(), source, budget)
    }

    /// Runs `source` as the init module `name` (errors labelled `label`), in a new runtime whose
    /// imports come from `dirs` if there is none yet. False when it failed.
    fn run_init(&mut self, name: &str, label: &str, dirs: ModuleDirs, source: &str, budget: Duration) -> bool {
        if self.scripts.is_none() {
            match Scripts::new(budget, dirs) {
                Ok(s) => self.scripts = Some(s),
                Err(m) => {
                    self.script_error(ScriptError { bar: m.clone(), full: m });
                    return false;
                }
            }
        }
        let start = Instant::now();
        self.script_sync();
        let Some(s) = self.scripts.as_mut() else { return false };
        let r = s.eval_module(name, label, source);
        s.held.extend(r.lines);
        self.effects
            .push(Effect::Log(format!("{label}: ran in {} ms", start.elapsed().as_millis())));
        match r.error {
            Some(e) => {
                self.script_error(e);
                false
            }
            None => true,
        }
    }

    /// C-SOURCE `:source`: the scripts again, clean (SCRIPTING.md ":source"). The old runtime goes
    /// with every command, mapping and `ready` handler (and the lines a call queued: none are left
    /// between calls); a new one runs the same init file (its modules read again). `ready` is not fired again.
    pub(crate) fn source(&mut self) {
        if self.script_lines > 0 {
            self.script_error(ScriptError {
                bar: "web.cmd: :source: a script cannot reload itself; type :source or map a key to it".to_string(),
                full: "web.cmd: :source: refused: a reload from inside a script call would drop the code that is running".to_string(),
            });
            return;
        }
        let files = self.script_files.clone();
        let budget = self.script_budget;
        // No runtime (no file at start): `ready` is long past, never fire it for a late file.
        let ready_fired = self.scripts.as_ref().is_none_or(|s| s.ready_fired);
        self.scripts = None;
        // Keys waiting for a mapping are matched again against the new mappings (the loop that
        // called us may still be draining them); with none waiting, no timer is left running.
        self.stop_idle_map_timer();
        let (ran, failed) = self.load_scripts_with(&files, budget);
        self.effects
            .push(Effect::Log(format!("script: sourced {ran} file(s), {failed} failed")));
        if failed == 0 {
            self.message = format!("sourced {ran} file(s)");
        }
        let Some(s) = self.scripts.as_mut() else { return };
        let held = if ready_fired {
            s.ready_fired = true;
            std::mem::take(&mut s.held)
        } else {
            Vec::new()
        };
        // What the file queued at their top level: with the host connected there is nothing to wait for.
        self.run_script_lines(held);
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
            self.script_sync();
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
        self.script_sync();
        let Some(s) = self.scripts.as_mut() else { return true };
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

    /// C-SCRIPT-BUFLIST: before every call into JS, the snapshot `web.buffers.list()` reads: the live
    /// buffers (the current one saved first, so its URL and title are fresh). Empty until `ready`.
    pub(crate) fn script_sync(&mut self) {
        if !self.scripts.as_ref().is_some_and(|s| s.ready_fired) {
            return;
        }
        self.save_current();
        let infos = crate::buffers::infos(&self.buffers);
        if let Some(s) = self.scripts.as_mut() {
            s.set_buffers(infos);
        }
    }

    /// After a call into JS: its error, then the lines it queued.
    pub(crate) fn finish_script_call(&mut self, r: CallResult) {
        if let Some(e) = r.error {
            self.script_error(e);
        }
        self.run_script_lines(r.lines);
    }

    /// `web.cmd`'s lines, each as typed after `:` (a chain past the depth limit drops the rest).
    ///
    /// C-CMDRESULT: after each line its promise is settled with `{ ok, message }` (`ok` is false when
    /// the command called `fail` or a script error came out of it; `message` is the text it gave the
    /// bar, "" when none). The settle is a call into JS of its own; the lines an `await` continuation
    /// queues there join the end of this list, as a promise reaction would run after what is queued.
    fn run_script_lines(&mut self, lines: Vec<ScriptLine>) {
        if self.script_lines == 0 {
            self.script_chain = 0;
        }
        self.script_lines += 1;
        let mut work: std::collections::VecDeque<ScriptLine> = lines.into();
        while let Some(ScriptLine { line, resolve }) = work.pop_front() {
            if self.scripts.as_ref().is_some_and(|s| s.overflow) {
                break;
            }
            self.script_chain += 1;
            if self.script_chain > SCRIPT_CHAIN_MAX {
                if let Some(s) = self.scripts.as_mut() {
                    s.overflow = true;
                }
                self.script_error(ScriptError {
                    bar: format!("web.cmd: more than {SCRIPT_CHAIN_MAX} lines in one chain; stopped"),
                    full: format!(
                        "web.cmd: the chain of lines run from one call passed SCRIPT_CHAIN_MAX ({SCRIPT_CHAIN_MAX}); it was stopped"
                    ),
                });
                break;
            }
            self.effects.push(Effect::Log(format!("script: :{line}")));
            let before = std::mem::take(&mut self.message);
            let outer_failed = std::mem::replace(&mut self.cmd_failed, false);
            self.run_command_line(&line);
            let failed = std::mem::replace(&mut self.cmd_failed, outer_failed);
            let message = self.message.clone();
            if message.is_empty() {
                self.message = before;
            }
            let Some(resolve) = resolve else { continue };
            self.script_sync();
            let Some(s) = self.scripts.as_mut() else { continue };
            let r = s.settle(resolve, !failed, message);
            if let Some(e) = r.error {
                // An error in the continuation is not the finished line's failure.
                let f = self.cmd_failed;
                self.script_error(e);
                self.cmd_failed = f;
            }
            work.extend(r.lines);
        }
        self.script_lines -= 1;
        if self.script_lines == 0 {
            if let Some(s) = self.scripts.as_mut() {
                s.overflow = false;
            }
        }
    }

    /// A script's error: the bar (kept until the user's next key, `page_loaded`) and the log.
    fn script_error(&mut self, e: ScriptError) {
        self.cmd_failed = true;
        self.effects.push(Effect::Log(format!("script error: {}", e.full)));
        self.sticky = e.bar.clone();
        self.message = e.bar;
    }

    /// C-ECHO `:echo TEXT`: TEXT in the bar (kept until the user's next key, as a script's error)
    /// and in the log.
    pub(crate) fn echo(&mut self, text: &str) {
        self.effects.push(Effect::Log(format!("echo: {text}")));
        self.sticky = text.to_string();
        self.message = text.to_string();
    }

    /// A page finished loading: the last message goes, unless it is one the user must see first
    /// (a script's error, an echo: until the next key press).
    pub fn page_loaded(&mut self) {
        if self.sticky.is_empty() || self.message != self.sticky {
            self.message.clear();
        }
    }
}
