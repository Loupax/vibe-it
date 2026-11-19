//! QY53f1: the layout build's parsed-rule cache, keyed per stylesheet piece
//! (each `<style>` element's effective text, plus the external/adopted CSS as
//! the last piece) instead of one concatenated string. An unchanged document
//! is a hit (no string is built, `<style>` text is compared in place); an
//! inserted, edited or removed `<style>` re-parses only that piece and
//! splices its rules into the cached list (same idea as `js_style::StyleCache`).
//!
//! Full re-parse fallbacks: any piece containing `@layer` (layer ranks are
//! numbered across sheets, so the pieces are parsed as one text), a changed
//! piece next to an `@layer` piece, a different `js_enabled()`, and a rule
//! list still shared by someone else (`Rc::get_mut` fails). `@import` needs no
//! fallback: the parser skips it, so it has no order effect.
use super::*;

struct Piece {
    /// The piece's effective text (the cache key).
    text: String,
    /// Rules this piece contributed (0 in concat mode).
    count: usize,
    layer: bool,
    mask: u8,
    kf: KeyframesRegistry,
    /// QMEM3e: some selector uses `:has(` / reads earlier siblings (what `getComputedStyle` needs).
    has: bool,
    sib: bool,
}

/// QMEM3e: what the caller's extra (external + adopted) CSS is.
#[derive(Clone, Copy)]
pub(crate) enum Extra<'a> {
    /// The text itself (the layout build).
    Text(&'a str),
    /// `js_style::css_ver()`: the text is `js_style::page_css()`, built only if the version moved.
    Ver((u64, u64)),
}

pub(crate) struct RulesCache {
    js: bool,
    ua_count: usize,
    ua_mask: u8,
    /// One per `<style>` element in document order, then the external CSS.
    pieces: Vec<Piece>,
    /// Some piece has `@layer`: `rules` came from parsing the joined text.
    concat: bool,
    rules: Rc<Vec<StyleRule>>,
    mask: u8,
    kf: Rc<KeyframesRegistry>,
    /// Bumped whenever `rules` changes (a rule-list identity that does not hold the `Rc`).
    generation: u64,
    /// `Some(v)`: the last piece equals `page_css()` at css version `v`.
    extra_ver: Option<(u64, u64)>,
    ua_flags: (bool, bool),
}

thread_local! {
    static GENERATION: Cell<u64> = const { Cell::new(0) };
}

fn next_generation() -> u64 {
    GENERATION.with(|g| {
        g.set(g.get() + 1);
        g.get()
    })
}

/// QMEM3e: the current rule list with its generation and `(has, sib)` flags, for
/// `StyleCache`. `StyleCache` does not keep the `Rc` (a second owner would force
/// every layout-side splice into a full re-parse).
pub(crate) fn current() -> Option<(Rc<Vec<StyleRule>>, u64, bool, bool)> {
    RULES_CACHE.with(|c| {
        c.borrow().as_ref().map(|rc| {
            let has = rc.ua_flags.0 || rc.pieces.iter().any(|p| p.has);
            let sib = rc.ua_flags.1 || rc.pieces.iter().any(|p| p.sib);
            (rc.rules.clone(), rc.generation, has, sib)
        })
    })
}

#[cfg(test)]
thread_local! {
    /// Test seam: stylesheet pieces parsed by the layout build so far.
    pub(crate) static PIECES_PARSED: Cell<usize> = const { Cell::new(0) };
    /// Test seam: full re-parses by the layout build so far.
    pub(crate) static FULL_PARSES: Cell<usize> = const { Cell::new(0) };
}

impl RulesCache {
    /// QOBS7: `(entries, approx bytes)`: the live pieces' text plus the rule list (QY53f1c: plus each
    /// rule's block text or parsed declarations). Only pieces that exist now are held.
    pub(crate) fn mem(&self) -> (usize, usize) {
        let text: usize = self.pieces.iter().map(|p| p.text.capacity()).sum();
        let heap: usize = self.rules.iter().map(StyleRule::heap_bytes).sum();
        (self.rules.len(), text + self.rules.capacity() * std::mem::size_of::<StyleRule>() + heap)
    }

    #[cfg(test)]
    pub(crate) fn piece_count(&self) -> usize {
        self.pieces.len()
    }
}

fn has_layer(t: &str) -> bool {
    let b = t.as_bytes();
    b.len() >= 6 && b.windows(6).any(|w| w[0] == b'@' && w[1..].eq_ignore_ascii_case(b"layer"))
}

/// Runs `f` and returns the state-pseudo-class mask recorded by the parse it did.
fn with_mask<T>(f: impl FnOnce() -> T) -> (T, u8) {
    let saved = state_rules_mask();
    STATE_RULES.with(|m| m.set(0));
    let r = f();
    let m = state_rules_mask();
    STATE_RULES.with(|c| c.set(saved | m));
    (r, m)
}

/// Parses `text` and appends its rules to `out` (no intermediate list to move: QY53f1b).
fn parse_piece(text: String, out: &mut Vec<StyleRule>) -> Piece {
    #[cfg(test)]
    PIECES_PARSED.with(|c| c.set(c.get() + 1));
    let n0 = out.len();
    let (kf, mask) = with_mask(|| parse_stylesheet_into(&text, out));
    let layer = has_layer(&text);
    let (has, sib) = crate::js_style::rule_flags(&out[n0..]);
    Piece { count: out.len() - n0, text, layer, mask, kf, has, sib }
}

/// Whether the node's effective text equals `key`, without building a string
/// unless the element has a CSSOM override.
fn node_matches(h: &Handle, key: &str) -> bool {
    fn walk(h: &Handle, key: &[u8], pos: &mut usize) -> bool {
        if let NodeData::Text { contents } = &h.data {
            let c = contents.borrow();
            let c = c.as_bytes();
            if !key[*pos..].starts_with(c) {
                return false;
            }
            *pos += c.len();
        }
        h.children.borrow().iter().all(|ch| walk(ch, key, pos))
    }
    if crate::images::has_style_override(Rc::as_ptr(h) as usize) {
        return crate::images::effective_style_text(h) == key;
    }
    let mut pos = 0;
    walk(h, key.as_bytes(), &mut pos) && pos == key.len()
}

fn merged_keyframes(pieces: &[Piece]) -> Rc<KeyframesRegistry> {
    let mut reg = KeyframesRegistry::new();
    for p in pieces {
        for (name, v) in &p.kf {
            reg.entry(name.clone()).or_default().extend(v.iter().cloned());
        }
    }
    Rc::new(reg)
}

fn finish_hit(rc: &RulesCache) -> Rc<Vec<StyleRule>> {
    note_state_rule(rc.mask);
    set_keyframes(rc.kf.clone());
    rc.rules.clone()
}

/// The parsed rules for `document`'s `<style>` elements followed by `extra_css`
/// (UA rules first), re-parsing only what changed since the previous call.
pub(crate) fn cached_rules(document: &Handle, extra_css: &str) -> Rc<Vec<StyleRule>> {
    cached_rules_for(document, Extra::Text(extra_css))
}

pub(crate) fn cached_rules_for(document: &Handle, extra: Extra) -> Rc<Vec<StyleRule>> {
    let built = std::cell::OnceCell::<String>::new();
    let extra_css = || -> &str {
        match extra {
            Extra::Text(t) => t,
            Extra::Ver(_) => built.get_or_init(crate::js_style::page_css).as_str(),
        }
    };
    let new_ver = match extra {
        Extra::Ver(v) => Some(v),
        Extra::Text(_) => None,
    };
    let mut nodes = Vec::new();
    crate::images::collect_style_nodes(document, &mut nodes);
    let js = js_enabled();
    RULES_CACHE.with(|c| {
        let mut slot = c.borrow_mut();
        if let Some(rc) = slot.as_mut()
            && rc.js == js
        {
            let n = nodes.len();
            let old = rc.pieces.len() - 1;
            let ex_same = match (extra, rc.extra_ver) {
                (Extra::Ver(v), Some(o)) if v == o => true,
                _ => rc.pieces[old].text == extra_css(),
            };
            let mut p = 0;
            while p < old.min(n) && node_matches(&nodes[p], &rc.pieces[p].text) {
                p += 1;
            }
            let mut s = 0;
            if ex_same {
                while s < old - p && s < n - p && node_matches(&nodes[n - 1 - s], &rc.pieces[old - 1 - s].text) {
                    s += 1;
                }
            }
            if ex_same && new_ver.is_some() {
                rc.extra_ver = new_ver;
            }
            if ex_same && p == old && n == old {
                return finish_hit(rc);
            }
            if !rc.concat {
                let old_end = if ex_same { old - s } else { old + 1 };
                let mut texts: Vec<String> = nodes[p..n - s].iter().map(crate::images::effective_style_text).collect();
                if !ex_same {
                    texts.push(extra_css().to_string());
                }
                let layered = rc.pieces[p..old_end].iter().any(|x| x.layer) || texts.iter().any(|t| has_layer(t));
                if !layered {
                    // The index/derived data hold a clone of the list; drop them so it is uniquely owned.
                    RULE_DERIVED.with(|d| *d.borrow_mut() = None);
                    if let Some(v) = Rc::get_mut(&mut rc.rules) {
                        let start = rc.ua_count + rc.pieces[..p].iter().map(|x| x.count).sum::<usize>();
                        let rem = rc.pieces[p..old_end].iter().map(|x| x.count).sum::<usize>();
                        let mut new_pieces = Vec::new();
                        let mut new_rules = Vec::new();
                        for t in texts {
                            new_pieces.push(parse_piece(t, &mut new_rules));
                        }
                        // Grow by exactly what is needed (a doubling Vec would double the cache's memory).
                        v.reserve_exact(new_rules.len().saturating_sub(rem));
                        v.splice(start..start + rem, new_rules);
                        rc.pieces.splice(p..old_end, new_pieces);
                        rc.mask = rc.ua_mask | rc.pieces.iter().fold(0, |a, x| a | x.mask);
                        rc.kf = merged_keyframes(&rc.pieces);
                        rc.generation = next_generation();
                        if !ex_same {
                            rc.extra_ver = new_ver;
                        }
                        return finish_hit(rc);
                    }
                }
            }
        }
        // Full parse.
        #[cfg(test)]
        FULL_PARSES.with(|c| c.set(c.get() + 1));
        let mut texts: Vec<String> = nodes.iter().map(crate::images::effective_style_text).collect();
        texts.push(extra_css().to_string());
        let ((mut rules, ua_count), ua_mask) = with_mask(|| {
            let mut rules = parse_ua_stylesheet(DEFAULT_CSS);
            if js {
                rules.extend(parse_ua_stylesheet("noscript { display: none; }"));
            }
            // QM1n1: always a replaced box (inert placeholder without media/libmpv).
            rules.extend(parse_ua_stylesheet("video { display: inline-block; object-fit: contain; } audio[controls] { display: inline-block; }"));
            let n = rules.len();
            (rules, n)
        });
        let ua_flags = crate::js_style::rule_flags(&rules);
        let concat = texts.iter().any(|t| has_layer(t));
        let mut pieces = Vec::with_capacity(texts.len());
        let (mask, kf);
        let concat_flags;
        if concat {
            // Layer ranks are numbered across sheets: parse the joined text as one.
            let mut src = String::new();
            for t in &texts {
                src.push_str(t);
                src.push('\n');
            }
            let n0 = rules.len();
            let (k, m) = with_mask(|| parse_stylesheet_into(&src, &mut rules));
            concat_flags = crate::js_style::rule_flags(&rules[n0..]);
            mask = ua_mask | m;
            kf = Rc::new(k);
            for t in texts {
                pieces.push(Piece { text: t, count: 0, layer: true, mask: 0, kf: KeyframesRegistry::new(), has: concat_flags.0, sib: concat_flags.1 });
            }
        } else {
            for t in texts {
                pieces.push(parse_piece(t, &mut rules));
            }
            mask = ua_mask | pieces.iter().fold(0, |a, x| a | x.mask);
            kf = merged_keyframes(&pieces);
        }
        let rc = RulesCache {
            js,
            ua_count,
            ua_mask,
            pieces,
            concat,
            rules: Rc::new({
                rules.shrink_to_fit();
                rules
            }),
            mask,
            kf,
            generation: next_generation(),
            extra_ver: new_ver,
            ua_flags,
        };
        let out = finish_hit(&rc);
        *slot = Some(rc);
        out
    })
}


#[cfg(test)]
pub(crate) fn rule_count() -> usize {
    RULES_CACHE.with(|c| c.borrow().as_ref().map_or(0, |r| r.rules.len()))
}
