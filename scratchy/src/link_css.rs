//! QY54b1: `<link rel=stylesheet>` elements that change after the page loaded (a script inserts or
//! removes one, or writes its `href`/`rel`/`disabled`). The parsed links' sheets are fetched before the
//! first layout (`net::fetch_stylesheets`) and concatenated into `App::extra_css`; this keeps what that
//! text is made of (one range per link, in document order), so a change rebuilds it from the links
//! that are in the document NOW: known sheets are reused, new hrefs are fetched through wire (async,
//! `AppEvent::LinkCss`), removed or disabled links drop out. One `extra_css` swap per change (one
//! `css_ver` bump for the JS thread's `page_css`, one restyle for the App build). Each link element
//! gets one `load` (or `error`) per href once its sheet is settled; the links of the parsed page are
//! marked as already fired (they never got events before either).
//!
//! Sheets are keyed by the link's resolved URL (`link_key`). An URL that an earlier link already
//! applies is not applied again and that second link fires nothing (both differ from browsers, which
//! apply the copy at its own place and fire `load`): YouTube watch parses `www-player.css` and then
//! inserts a second link to it, and its `load` makes the player re-measure a box that scratchy lays
//! out 0 px tall (an absolutely positioned `top: 0; bottom: 0` box keeps its content height; see
//! TODO.txt), so the player would vanish.
use super::*;
use std::ops::Range;

/// What a parsed page's `extra_css` is made of: each stylesheet link's key (`link_key`) with the byte
/// range of its text (without the `\n` that follows it), `None` when its fetch failed. In document
/// order, each key once.
pub(crate) type SheetParts = Vec<(String, Option<Range<usize>>)>;

/// The key a stylesheet link's sheet is known by: its `href` resolved against `base` (a `data:` URI
/// or an `href` that cannot be resolved stays as written, trimmed).
pub(crate) fn link_key(base: Option<&Url>, raw: &str) -> String {
    let t = raw.trim();
    if t.len() >= 5 && t[..5].eq_ignore_ascii_case("data:") {
        return t.to_string();
    }
    resolve_href(base, t).map(|u| u.to_string()).unwrap_or_else(|_| t.to_string())
}

/// The raw `href` of a stylesheet link that applies: `<link>` whose `rel` has the `stylesheet` token
/// (ASCII case-insensitive), with an `href`, and without `disabled`.
pub(crate) fn stylesheet_link_href(h: &Handle) -> Option<String> {
    let NodeData::Element { name, .. } = &h.data else { return None };
    if name.local.as_ref() != "link" {
        return None;
    }
    let is_sheet = element_attr(h, "rel").is_some_and(|r| r.split_ascii_whitespace().any(|t| t.eq_ignore_ascii_case("stylesheet")));
    if !is_sheet || element_attr(h, "disabled").is_some() {
        return None;
    }
    element_attr(h, "href").map(|s| s.to_string())
}

/// Every applying stylesheet link of `doc` with its key (`link_key`), in document order (the same walk
/// as `find_external_resources`: element children only, so template contents are not included).
pub(crate) fn stylesheet_links(doc: &Handle, base: Option<&Url>) -> Vec<(Handle, String)> {
    let mut out = Vec::new();
    let mut stack = vec![doc.clone()];
    while let Some(n) = stack.pop() {
        if let Some(href) = stylesheet_link_href(&n) {
            out.push((n.clone(), link_key(base, &href)));
        }
        stack.extend(n.children.borrow().iter().rev().cloned());
    }
    out
}

/// True when `h` is a `<link>` element or contains one.
pub(crate) fn has_link(h: &Handle) -> bool {
    let mut stack = vec![h.clone()];
    while let Some(n) = stack.pop() {
        if matches!(&n.data, NodeData::Element { name, .. } if name.local.as_ref() == "link") {
            return true;
        }
        stack.extend(n.children.borrow().iter().cloned());
    }
    false
}

/// The attributes whose change can change which sheets a `<link>` applies.
pub(crate) fn is_link_sheet_attr(h: &Handle, attr: &str) -> bool {
    matches!(attr, "href" | "rel" | "disabled") && matches!(&h.data, NodeData::Element { name, .. } if name.local.as_ref() == "link")
}

/// The stylesheet links of the current page and the sheets known for them.
#[derive(Default, Clone)]
pub(crate) struct LinkSheets {
    /// Leading bytes of `extra_css` whose links are not known (a page installed without `SheetParts`:
    /// a streamed page, `:help`, tests). Kept first, unchanged.
    opaque: usize,
    /// Hrefs that `opaque` already holds (not fetched again).
    covered: HashSet<String>,
    /// The rest of `extra_css`: (href, range of its text), in document order.
    parts: Vec<(String, Range<usize>)>,
    /// Sheets fetched for this page that no link uses right now (removed or disabled links).
    spare: HashMap<String, String>,
    failed: HashSet<String>,
    pending: HashSet<String>,
    /// Link elements that got their `load`/`error`, with the href it was for.
    fired: Vec<(WeakHandle, String)>,
}

/// What one `LinkSheets::sync` decided.
#[derive(Default)]
pub(crate) struct SyncOut {
    /// The new `extra_css`, when it changed.
    pub(crate) css: Option<String>,
    /// Hrefs to fetch now (already marked pending).
    pub(crate) fetch: Vec<String>,
    /// Link elements to fire `load` (true) or `error` (false) at, in document order.
    pub(crate) fire: Vec<(Handle, bool)>,
}

impl LinkSheets {
    /// The state for a freshly installed `document` whose `extra_css` (`css_len` bytes) is `parts`
    /// (empty parts: the whole text is opaque). The links already in it count as fired.
    pub(crate) fn adopt(document: &Handle, base: Option<&Url>, css_len: usize, parts: SheetParts) -> Self {
        let mut s = LinkSheets::default();
        let links = stylesheet_links(document, base);
        if parts.is_empty() {
            s.opaque = css_len;
            s.covered = links.iter().map(|(_, href)| href.clone()).collect();
        }
        for (href, range) in parts {
            match range {
                Some(r) => s.parts.push((href, r)),
                None => {
                    s.failed.insert(href);
                }
            }
        }
        s.fired = links.iter().map(|(h, href)| (Rc::downgrade(h), href.clone())).collect();
        s
    }

    /// Whether a fetch for `href` is in flight.
    #[cfg(test)]
    pub(crate) fn is_pending(&self, href: &str) -> bool {
        self.pending.contains(href)
    }

    /// Records the outcome of fetching `href` (`None`: failed).
    pub(crate) fn settle(&mut self, href: &str, css: Option<String>) {
        if !self.pending.remove(href) {
            return;
        }
        match css {
            Some(t) => {
                self.spare.insert(href.to_string(), t);
            }
            None => {
                self.failed.insert(href.to_string());
            }
        }
    }

    /// Rebuilds `css` (the current `extra_css`) from the stylesheet links in `document` now
    /// (`base`: the page's base URL, as for `adopt`).
    pub(crate) fn sync(&mut self, document: &Handle, base: Option<&Url>, css: &str) -> SyncOut {
        let mut out = SyncOut::default();
        let links = stylesheet_links(document, base);
        let mut new = String::with_capacity(css.len());
        new.push_str(&css[..self.opaque.min(css.len())]);
        let mut parts = Vec::new();
        let mut settled: Vec<Option<bool>> = Vec::with_capacity(links.len());
        let mut seen: HashSet<&str> = HashSet::new();
        for (_, href) in &links {
            if self.covered.contains(href) {
                settled.push(Some(true));
                continue;
            }
            // A key an earlier link already applies: not applied again, no event (module docs).
            if !seen.insert(href.as_str()) {
                settled.push(None);
                continue;
            }
            let text = match self.parts.iter().find(|(k, _)| k == href) {
                Some((_, r)) => css.get(r.clone()),
                None => self.spare.get(href).map(String::as_str),
            };
            if let Some(t) = text {
                let start = new.len();
                new.push_str(t);
                parts.push((href.clone(), start..new.len()));
                new.push('\n');
                settled.push(Some(true));
            } else if self.failed.contains(href) {
                settled.push(Some(false));
            } else {
                if self.pending.insert(href.clone()) {
                    out.fetch.push(href.clone());
                }
                settled.push(None);
            }
        }
        if new != css {
            // Sheets no link uses any more stay known (a link put back, `disabled` turned off).
            for (k, r) in &self.parts {
                if !parts.iter().any(|(n, _)| n == k)
                    && let Some(t) = css.get(r.clone())
                {
                    self.spare.entry(k.clone()).or_insert_with(|| t.to_string());
                }
            }
            for (k, _) in &parts {
                self.spare.remove(k);
            }
            self.parts = parts;
            // QMEM3b: the string lives as long as the page.
            new.shrink_to_fit();
            out.css = Some(new);
        }
        self.fired.retain(|(w, _)| w.strong_count() > 0);
        for ((h, href), ok) in links.into_iter().zip(settled) {
            let Some(ok) = ok else { continue };
            match self.fired.iter_mut().find(|(w, _)| w.upgrade().is_some_and(|x| Rc::ptr_eq(&x, &h))) {
                Some((_, done)) if *done == href => continue,
                Some((_, done)) => *done = href,
                None => self.fired.push((Rc::downgrade(&h), href)),
            }
            out.fire.push((h, ok));
        }
        out
    }
}

/// Fetches one stylesheet `href` (resolved against `base`) and expands its `@import`s, as the parsed
/// links are (`net::fetch_stylesheets`). Runs off the main thread.
pub(crate) fn fetch_link_sheet(socket: &std::path::Path, base: &Url, top: &Url, href: &str) -> Option<String> {
    fetch_one_stylesheet(socket, base, top, href)
}

/// Applies `out` of a `LinkSheets::sync`: the new CSS goes to `extra_css` and the JS thread's
/// `page_css`, then the settled links get their `load`/`error`. Returns whether the CSS changed.
pub(crate) fn apply_sync(out: &mut SyncOut, extra_css: &mut String, js: Option<&JsEngine>) -> bool {
    let changed = match out.css.take() {
        Some(css) => {
            crate::js_style::set_js_extra_css(&css);
            *extra_css = css;
            true
        }
        None => false,
    };
    if let Some(js) = js {
        for (h, ok) in std::mem::take(&mut out.fire) {
            js.dispatch_event(Some(&h), if ok { "load" } else { "error" }, &[]);
        }
    }
    changed
}
