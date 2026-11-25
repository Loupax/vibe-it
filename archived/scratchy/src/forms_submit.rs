//! Q30e: form submission.
//!
//! DESIGN NOTE (the user will redesign forms later, keep this easy to change):
//! everything here up to `plan_submission` is PURE (no `App`, no network): the
//! form data set (`form_data_set`), its three encoders and the submission plan
//! (method, final URL, body). The `App` side (`impl App` at the end) only fires
//! the `submit` event, calls the plan and starts a navigation.
//!
//! What is implemented: successful controls (named, not disabled; text-like and
//! textarea LIVE values with CRLF newlines; checked checkboxes/radios, value
//! default `on`; selected options; hidden; the submitter button; `type=image`
//! as `name.x`/`name.y` = 0; `<input type=file>` as an empty file part), the
//! `form=` owner attribute, `formaction/formmethod/formenctype/formtarget` on
//! the submitter, GET (query replaced), POST urlencoded / multipart /
//! `text/plain`, `method=dialog`.
//!
//! What is NOT: constraint validation of any kind (`required`, `pattern`,
//! `novalidate` are all ignored: a submission is never blocked), `accept-charset`
//! (always UTF-8), real file contents, the click position of `type=image`.

use super::*;

/// One entry of the form data set. Names and text values already have CRLF
/// newlines (spec: "normalize line breaks" at encoding time).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EntryValue {
    Text(String),
    /// A file control with nothing chosen (the file chooser is not
    /// implemented): an empty file part named `filename`, empty for "none".
    File { filename: String, mime: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) name: String,
    pub(crate) value: EntryValue,
}

fn crlf(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push_str("\r\n");
            }
            '\n' => out.push_str("\r\n"),
            c => out.push(c),
        }
    }
    out
}

/// The collapsed, trimmed text of an `<option>` (its value when it has no
/// `value` attribute).
fn option_text(o: &Handle) -> String {
    let mut t = String::new();
    collect_text_only(o, &mut t);
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn option_value(o: &Handle) -> String {
    attr_string(o, "value").unwrap_or_else(|| option_text(o))
}

/// The `<option>`s of a select in tree order, with whether an enclosing
/// `<optgroup disabled>` or its own `disabled` disables it.
fn select_options_all(select: &Handle) -> Vec<(Handle, bool)> {
    fn walk(h: &Handle, group_disabled: bool, out: &mut Vec<(Handle, bool)>) {
        for c in h.children.borrow().iter() {
            match element_tag(c) {
                "option" => out.push((c.clone(), group_disabled || element_attr(c, "disabled").is_some())),
                "optgroup" => walk(c, group_disabled || element_attr(c, "disabled").is_some(), out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(select, false, &mut out);
    out
}

/// The selected options of a `<select>`: the ONE place that reads the live
/// selection (Q30g, forms_select.rs): the user's/script's choice, else the
/// `selected` attributes with the single-select default rules.
pub(crate) fn selected_options(select: &Handle) -> Vec<Handle> {
    selected_option_handles(select)
}

/// The submittable elements owned by `form`, in tree order.
fn form_controls(form: &Handle) -> Vec<Handle> {
    fn walk(h: &Handle, form: &Handle, out: &mut Vec<Handle>) {
        if matches!(element_tag(h), "input" | "button" | "select" | "textarea") && form_owner(h).is_some_and(|f| Rc::ptr_eq(&f, form)) {
            out.push(h.clone());
        }
        for c in h.children.borrow().iter() {
            walk(c, form, out);
        }
    }
    let mut out = Vec::new();
    walk(&tree_root_of(form), form, &mut out);
    out
}

fn tree_root_of(h: &Handle) -> Handle {
    let mut top = h.clone();
    while let Some(p) = parent_handle(&top) {
        top = p;
    }
    top
}

fn inside_datalist(h: &Handle) -> bool {
    let mut cur = parent_handle(h);
    while let Some(n) = cur {
        if element_tag(&n) == "datalist" {
            return true;
        }
        cur = parent_handle(&n);
    }
    false
}

/// The default value of the non-text value controls (`range`, `color`, dates).
fn simple_input_value(h: &Handle, ty: &str) -> String {
    let v = attr_string(h, "value");
    match ty {
        "color" => v.filter(|c| c.len() == 7 && c.starts_with('#')).map(|c| c.to_ascii_lowercase()).unwrap_or_else(|| "#000000".to_string()),
        "range" => {
            let f = |n: &str| attr_string(h, n).and_then(|s| s.trim().parse::<f64>().ok());
            let (min, max) = (f("min").unwrap_or(0.0), f("max").unwrap_or(100.0));
            let val = v.and_then(|s| s.trim().parse::<f64>().ok()).unwrap_or(min + (max - min) / 2.0);
            let val = val.clamp(min, max.max(min));
            if val.fract() == 0.0 { format!("{}", val as i64) } else { format!("{val}") }
        }
        _ => v.unwrap_or_default(),
    }
}

/// Whether `h` is a submit button (`<button>` default type, `input submit|image`).
pub(crate) fn is_submit_button(h: &Handle) -> bool {
    match element_tag(h) {
        "button" => !matches!(attr_string(h, "type").map(|t| t.trim().to_ascii_lowercase()).as_deref(), Some("reset" | "button")),
        "input" => matches!(input_type(h).as_str(), "submit" | "image"),
        _ => false,
    }
}

/// The form data set of `form` (HTML "constructing the entry list").
/// `submitter` is the button that triggered the submission, if any.
pub(crate) fn form_data_set(form: &Handle, submitter: Option<&Handle>) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    let mut push = |name: &str, v: EntryValue| {
        let value = match v {
            EntryValue::Text(t) => EntryValue::Text(crlf(&t)),
            f => f,
        };
        out.push(Entry { name: crlf(name), value });
    };
    for h in form_controls(form) {
        if is_disabled_control(&h) || inside_datalist(&h) {
            continue;
        }
        let tag = element_tag(&h).to_string();
        let name = attr_string(&h, "name").unwrap_or_default();
        let is_submitter = submitter.is_some_and(|s| Rc::ptr_eq(s, &h));
        match tag.as_str() {
            "input" => {
                let ty = input_type(&h);
                match ty.as_str() {
                    "image" => {
                        if is_submitter {
                            let (x, y) = if name.is_empty() { ("x".to_string(), "y".to_string()) } else { (format!("{name}.x"), format!("{name}.y")) };
                            push(&x, EntryValue::Text("0".into()));
                            push(&y, EntryValue::Text("0".into()));
                        }
                        continue;
                    }
                    "submit" => {
                        if is_submitter && !name.is_empty() {
                            push(&name, EntryValue::Text(attr_string(&h, "value").unwrap_or_else(|| "Submit".to_string())));
                        }
                        continue;
                    }
                    "reset" | "button" => continue,
                    _ => {}
                }
                if name.is_empty() {
                    continue;
                }
                match ty.as_str() {
                    "checkbox" | "radio" => {
                        if live_checked(&h) {
                            push(&name, EntryValue::Text(attr_string(&h, "value").unwrap_or_else(|| "on".to_string())));
                        }
                    }
                    "file" => push(&name, EntryValue::File { filename: String::new(), mime: "application/octet-stream".into() }),
                    "hidden" => {
                        if name == "_charset_" {
                            push(&name, EntryValue::Text("UTF-8".into()));
                        } else {
                            push(&name, EntryValue::Text(attr_string(&h, "value").unwrap_or_default()));
                        }
                    }
                    _ => {
                        let v = if text_control_kind(&h).is_some() { text_value(&h) } else { simple_input_value(&h, &ty) };
                        push(&name, EntryValue::Text(v));
                    }
                }
            }
            "button" => {
                if is_submitter && !name.is_empty() {
                    push(&name, EntryValue::Text(attr_string(&h, "value").unwrap_or_default()));
                }
            }
            "textarea" => {
                if !name.is_empty() {
                    push(&name, EntryValue::Text(text_value(&h)));
                }
            }
            "select" => {
                if name.is_empty() {
                    continue;
                }
                let opts = select_options_all(&h);
                for o in selected_options(&h) {
                    let disabled = opts.iter().any(|(p, d)| Rc::ptr_eq(p, &o) && *d);
                    if !disabled {
                        push(&name, EntryValue::Text(option_value(&o)));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// `application/x-www-form-urlencoded`. Files contribute their (empty) file
/// name, as browsers do.
pub(crate) fn encode_urlencoded(entries: &[Entry]) -> String {
    let mut s = url::form_urlencoded::Serializer::new(String::new());
    for e in entries {
        match &e.value {
            EntryValue::Text(t) => s.append_pair(&e.name, t),
            EntryValue::File { filename, .. } => s.append_pair(&e.name, filename),
        };
    }
    s.finish()
}

fn multipart_escape(s: &str) -> String {
    s.replace('\r', "%0D").replace('\n', "%0A").replace('"', "%22")
}

/// `multipart/form-data` with the given boundary (no leading dashes needed).
pub(crate) fn encode_multipart(entries: &[Entry], boundary: &str) -> Vec<u8> {
    let mut out = String::new();
    for e in entries {
        out.push_str(&format!("--{boundary}\r\n"));
        match &e.value {
            EntryValue::Text(t) => {
                out.push_str(&format!("Content-Disposition: form-data; name=\"{}\"\r\n\r\n{t}\r\n", multipart_escape(&e.name)));
            }
            EntryValue::File { filename, mime } => {
                out.push_str(&format!(
                    "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: {mime}\r\n\r\n\r\n",
                    multipart_escape(&e.name),
                    multipart_escape(filename)
                ));
            }
        }
    }
    out.push_str(&format!("--{boundary}--\r\n"));
    out.into_bytes()
}

/// `text/plain`: `name=value` lines (no escaping: the spec says it is not
/// reliably parseable).
pub(crate) fn encode_text_plain(entries: &[Entry]) -> String {
    let mut out = String::new();
    for e in entries {
        let v = match &e.value {
            EntryValue::Text(t) => t.as_str(),
            EntryValue::File { filename, .. } => filename.as_str(),
        };
        out.push_str(&format!("{}={v}\r\n", e.name));
    }
    out
}

/// A multipart boundary no entry can contain in practice (time + counter).
fn new_boundary() -> String {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mix = t.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ n.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    format!("----ScratchyFormBoundary{mix:016x}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubmitMethod {
    Get,
    Post,
    Dialog,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PostBody {
    pub(crate) content_type: String,
    pub(crate) body: Vec<u8>,
}

/// What a submission will do: the URL to navigate to and, for POST, the body.
#[derive(Debug, Clone)]
pub(crate) struct SubmitPlan {
    pub(crate) method: SubmitMethod,
    pub(crate) url: Url,
    pub(crate) post: Option<PostBody>,
    /// The effective `target` (lower-case, empty = `_self`).
    pub(crate) target: String,
}

/// The submitter's own attribute (`formaction`...) when it is a submit button
/// of this form, else the form's attribute.
fn submit_attr(form: &Handle, submitter: Option<&Handle>, button_attr: &str, form_attr: &str) -> Option<String> {
    if let Some(s) = submitter.filter(|s| is_submit_button(s))
        && let Some(v) = attr_string(s, button_attr)
    {
        return Some(v);
    }
    attr_string(form, form_attr)
}

pub(crate) fn submit_method_of(form: &Handle, submitter: Option<&Handle>) -> SubmitMethod {
    match submit_attr(form, submitter, "formmethod", "method").map(|m| m.trim().to_ascii_lowercase()).as_deref() {
        Some("post") => SubmitMethod::Post,
        Some("dialog") => SubmitMethod::Dialog,
        _ => SubmitMethod::Get,
    }
}

fn submit_enctype(form: &Handle, submitter: Option<&Handle>) -> &'static str {
    match submit_attr(form, submitter, "formenctype", "enctype").map(|m| m.trim().to_ascii_lowercase()).as_deref() {
        Some("multipart/form-data") => "multipart/form-data",
        Some("text/plain") => "text/plain",
        _ => "application/x-www-form-urlencoded",
    }
}

/// Builds the plan of submitting `form` from the page at `page` (the base
/// the `action` resolves against; `None` for a page with no URL). `Err` is a
/// one-line reason (unresolvable action, unsupported scheme). `Dialog` has no
/// URL: the caller handles it before calling this.
pub(crate) fn plan_submission(form: &Handle, submitter: Option<&Handle>, page: Option<&Url>) -> Result<SubmitPlan, String> {
    let method = submit_method_of(form, submitter);
    let action = submit_attr(form, submitter, "formaction", "action").map(|a| a.trim().to_string()).filter(|a| !a.is_empty());
    let mut url = match (&action, page) {
        (Some(a), base) => resolve_href(base, a)?,
        (None, Some(p)) => p.clone(),
        (None, None) => return Err("form has no action and the page has no URL".to_string()),
    };
    if !matches!(url.scheme(), "http" | "https" | "file") {
        return Err(format!("form action scheme {}: is not supported", url.scheme()));
    }
    let target = submit_attr(form, submitter, "formtarget", "target").unwrap_or_default().trim().to_ascii_lowercase();
    let entries = form_data_set(form, submitter);
    if method == SubmitMethod::Dialog {
        return Ok(SubmitPlan { method, url, post: None, target });
    }
    if method == SubmitMethod::Get {
        let q = encode_urlencoded(&entries);
        url.set_query(Some(&q));
        return Ok(SubmitPlan { method, url, post: None, target });
    }
    let post = match submit_enctype(form, submitter) {
        "multipart/form-data" => {
            let b = new_boundary();
            PostBody { content_type: format!("multipart/form-data; boundary={b}"), body: encode_multipart(&entries, &b) }
        }
        "text/plain" => PostBody { content_type: "text/plain".into(), body: encode_text_plain(&entries).into_bytes() },
        _ => PostBody { content_type: "application/x-www-form-urlencoded".into(), body: encode_urlencoded(&entries).into_bytes() },
    };
    Ok(SubmitPlan { method, url, post: Some(post), target })
}

/// The request text for a navigation by form POST: `build_wire_request` plus
/// the cookie headers of `ctx`, a `Content-Type` and an `Origin` (browsers
/// send it on every form POST; `null` for a `file:` page).
pub(crate) fn build_post_request_ctx(url: &Url, ctx: &CookieContext, post: &PostBody, page: Option<&Url>) -> String {
    let mut headers = vec![("Content-Type".to_string(), post.content_type.clone())];
    let origin = page.map_or_else(|| "null".to_string(), origin_header);
    headers.push(("Origin".to_string(), origin));
    headers.extend(wire_cookie_headers(ctx));
    let bytes = build_wire_request("POST", url, &headers, &post.body).unwrap_or_default();
    // Every body above is built from Rust strings: valid UTF-8 end to end.
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The default button of `form`: its first submit button in tree order.
pub(crate) fn default_button(form: &Handle) -> Option<Handle> {
    form_controls(form).into_iter().find(is_submit_button)
}

/// Whether implicit submission (Enter in a text field) with no submit button
/// is allowed: at most one field that blocks it (a text-like input).
pub(crate) fn implicit_submission_ok(form: &Handle) -> bool {
    let n = form_controls(form)
        .iter()
        .filter(|c| {
            element_tag(c) == "input"
                && !matches!(input_type(c).as_str(), "submit" | "image" | "reset" | "button" | "checkbox" | "radio" | "hidden" | "file" | "color" | "range")
        })
        .count();
    n <= 1
}

impl App {
    /// A submit event for `form` (cancelable, `submitter` set), then the
    /// default action. Used by clicking a submit button and implicit
    /// submission; `form.requestSubmit()` runs the same steps in script.
    pub(crate) fn request_submit(&mut self, form: &Handle, submitter: Option<&Handle>) {
        let before = self.nav_generation;
        let mut init = vec![("bubbles", EventField::Bool(true)), ("cancelable", EventField::Bool(true))];
        if let Some(s) = submitter {
            init.push(("submitterId", EventField::Node(s.clone())));
        }
        if !self.js_dispatch(Some(form), "submit", &init) {
            return;
        }
        // A handler called `form.submit()` itself: it already navigated.
        if self.nav_generation != before {
            return;
        }
        self.form_navigate(form, submitter);
    }

    /// The default action of a submission (no events): `dialog` closes the
    /// dialog, GET/POST navigate this tab.
    pub(crate) fn form_navigate(&mut self, form: &Handle, submitter: Option<&Handle>) {
        let plan = match plan_submission(form, submitter, self.base_url.as_ref()) {
            Ok(p) => p,
            Err(e) => {
                diag!("scratchy: form submission failed: {e}");
                self.show_message(&format!("scratchy: form submission failed: {e}"));
                return;
            }
        };
        if plan.method == SubmitMethod::Dialog {
            self.form_dialog_close(form, submitter);
            return;
        }
        if !matches!(plan.target.as_str(), "" | "_self" | "_top" | "_parent") {
            diag!("scratchy: form target={} opens in this tab (no tabs)", plan.target);
        }
        let method = if plan.post.is_some() { "POST" } else { "GET" };
        if let Some(page) = self.base_url.clone() {
            match RequestPolicy::default().check_request(&page, &plan.url, method, plan.post.is_some()) {
                Ok(outcome) => {
                    if let Some(class) = outcome.warning
                        && !is_same_origin(&page, &plan.url)
                    {
                        diag!("{}", format_net_warning(&page, &plan.url, class));
                    }
                }
                Err(e) => {
                    diag!("scratchy: form submission to {} blocked: {e}", plan.url);
                    self.show_message(&format!("scratchy: form submission blocked: {e}"));
                    return;
                }
            }
        }
        match plan.post {
            None => self.start_navigation(plan.url.as_str()),
            Some(post) => {
                diag!("scratchy: form POST {} ({}, {} bytes)", plan.url, post.content_type.split(';').next().unwrap_or(""), post.body.len());
                self.start_post_navigation(plan.url, post);
            }
        }
    }

    /// `method=dialog`: closes the `<dialog>` that contains the form; the
    /// submitter's `value` becomes its `returnValue`.
    fn form_dialog_close(&mut self, form: &Handle, submitter: Option<&Handle>) {
        let mut cur = parent_handle(form);
        while let Some(n) = cur {
            if element_tag(&n) == "dialog" {
                let rv = submitter.filter(|s| is_submit_button(s)).and_then(|s| attr_string(s, "value"));
                let init: Vec<(&str, EventField)> = rv.map(|v| vec![("rv", EventField::Str(v))]).unwrap_or_default();
                self.js_dispatch(Some(&n), "__dialogclose", &init);
                return;
            }
            cur = parent_handle(&n);
        }
    }

    /// A POST navigation: like `start_navigation` but with a request body.
    /// History gets the action URL; going back/forward/refresh to that entry
    /// re-GETs it (never re-POSTs).
    fn start_post_navigation(&mut self, url: Url, post: PostBody) {
        self.meta_chain = 0;
        self.pre_help_page = None;
        self.push_history(url.clone());
        self.nav_initiator = self.base_url.clone();
        self.pending_post = Some(post);
        self.visit_pending = true; // QCOMP3
        self.fetch_url_inner(url);
    }
}
