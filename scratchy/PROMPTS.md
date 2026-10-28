# scratchy — prompt log

Append-only log of the exact prompt that produced each feature, in the order
it was built.

---

## v1 — minimal from-scratch renderer

A Rust binary that reads HTML from a file or stdin (in full, batch-style —
not streamed), strips all markup, and displays the plain text content —
word-wrapped — in a native Wayland window. No CSS, no styling, no
networking, no links, no scrolling, no incremental/streaming rendering.

**Requirements:**
1. **Input:** if given one CLI argument, treat it as a file path and read
   HTML from it. If given zero arguments, read HTML from stdin until EOF,
   then proceed — this is a one-shot batch read, not a stream. More than one
   argument → print `usage: scratchy [file]` to stderr, exit 1. Scratchy
   never performs networking itself — fetching is the caller's job
   (`curl url | scratchy`, `nc ... | scratchy`, etc.).
2. **Parsing:** parse the fully-read input with `html5ever` into a DOM tree.
3. **Content extraction:** walk the DOM and collect visible text content in
   document order, with `<script>`/`<style>` contents excluded. No CSS
   parsing, no cascade, no selectors.
4. **Layout:** word-wrap the extracted text to the window's current width
   using one fixed font, one fixed size, one color, via `cosmic-text`
   (shaping, line-breaking, and glyph rasterization).
5. **Window:** a single `winit` window targeting Wayland natively, default
   size 1024x768, resizable. Presentation via `softbuffer` (CPU pixel buffer
   blit) — no GPU/wgpu, no `tiny-skia` yet.
6. **Overflow:** if content is taller than the window, it's simply clipped —
   no scrolling.
7. **Closing the window exits the process cleanly.**
8. **Explicitly out of scope, do not implement:** CSS of any kind,
   hyperlinks/navigation, scrolling, images, forms, JavaScript, networking,
   incremental/streaming parsing or rendering (reading stdin as it arrives,
   repainting as content streams in — that's the *next* feature, logged
   separately).
9. **Project setup:** Cargo binary crate `scratchy`. Confirm current crate
   names/versions for `winit`, `softbuffer`, `html5ever`, `cosmic-text`
   against crates.io rather than assuming.
10. No automated tests for v1 — behavior is directly observable.

**Deliverable:** `main.rs` + `Cargo.toml` implementing exactly the above.

---

## v2 — stream stdin

Rather than reading stdin to completion before any parsing happens, read it
incrementally and repaint as content arrives — the behavior deliberately
deferred in v1.

**Requirements:**
1. **File input is unchanged** — still read fully upfront, same as v1 (no
   streaming benefit for a local file that's already entirely present).
2. **Stdin input becomes incremental:** read stdin in fixed-size byte chunks
   (not line-based — HTML isn't guaranteed to be newline-delimited) on a
   background thread, rather than blocking the whole program on a single
   `read_to_end`.
3. **Incremental parsing:** feed each chunk into html5ever's parser as it
   arrives, using its lower-level incremental API (`process()` per chunk,
   `finish()` at EOF) instead of v1's one-shot `read_from()`.
4. **Wake the event loop on new data:** the background thread reading stdin
   must notify the `winit` event loop when a new chunk has been parsed,
   using an `EventLoopProxy`/user event — the event loop must not block
   waiting on stdin itself, since it also has to keep handling window
   events (resize, close).
5. **Redraw on each chunk:** after each chunk is parsed, re-extract text
   from the (partially-built) DOM from scratch and re-run layout — no
   incremental diffing of only the new nodes. Simplicity over premature
   optimization, same as v1's approach.
6. **No loading indicator, no progress UI** — that's chrome, out of scope.
7. **EOF just means no more updates arrive** — no special "done" state or
   signal to the user.
8. Everything still out of scope per v1 (CSS, links, scrolling, networking)
   stays out of scope here too — this feature is only about *when*
   parsing/painting happens, not *what* gets parsed or painted.

**Deliverable:** modifications to `scratchy`'s existing `main.rs`/
`Cargo.toml` implementing exactly the above.

---

## v3 — CSS parsing/cascade (color only)

**Reference material for this stage:**
- Selector syntax & specificity: CSS Selectors Level 4
  (https://www.w3.org/TR/selectors-4/)
- Cascade algorithm & inheritance: CSS Cascading and Inheritance Level 4
  (https://www.w3.org/TR/css-cascade-4/)
- Structural reference (read for approach, not copied): Servo's `style`
  crate, which solves this exact problem using the same
  `cssparser`/`selectors` crates this stage uses.

**Requirements:**
1. **Collect CSS source:** concatenate the text content of all `<style>`
   elements, in document order, into one stylesheet source. External
   stylesheets (`<link rel=stylesheet>`) stay out of scope — scratchy
   still never performs networking.
2. **Parse with `cssparser`** into simple qualified rules
   (`selector-list { declarations }`). Skip any at-rules encountered
   (`@media`, `@import`, `@font-face`, etc.) and any nested rules — out of
   scope.
3. From each declaration block, parse only the `color` property; every
   other declared property is parsed-and-discarded, not erred on.
4. **Implement `selectors`' `Element` trait** for our DOM node type:
   type/tag selectors, `#id`, `.class`, attribute selectors,
   descendant/child/sibling combinators. Interactive pseudo-classes
   (`:hover`, `:focus`, `:visited`) are out of scope — nothing is
   interactive yet.
5. Also parse each element's inline `style="..."` attribute for a `color`
   declaration, same parsing path.
6. **Resolve the cascade per element:** match all rules against it, sort
   by specificity then source order (later wins ties), apply the winner,
   then let an inline `style` attribute's `color` override that if present
   (inline always wins — no `!important` support needed since we don't
   implement `!important` at all).
7. **`color` inherits:** an element with no winning declaration takes its
   parent's computed color; the root default stays black, as in v1/v2.
8. **Rendering:** split the on-screen text into colored runs following
   each node's computed color, and paint each run with `cosmic-text`'s
   per-span attributes (`set_rich_text` or equivalent) instead of v1/v2's
   single uniform `set_text` call.
9. **Explicitly out of scope:** box model (margin/padding/border/
   background), `display`/layout, fonts/sizes/weights beyond color,
   `@media`/`@import`/`@font-face`, `!important`, multiple cascade
   origins, external stylesheets — all separate, later, logged features.
10. **Streaming is preserved:** as more stdin content and more `<style>`
    rules arrive, re-run cascade and repaint per chunk, same as v2
    already does.
11. **Project setup:** confirm current `cssparser`/`selectors` APIs
    against the installed crate source rather than assuming from
    memory — these two are lower-level and less stable than the v1/v2
    crates were.

**Deliverable:** modifications to `scratchy`'s existing `main.rs`/
`Cargo.toml` implementing exactly the above.

---

## v4 — box model & block/inline layout

**Reference material:**
- CSS Box Model Module Level 3 (https://www.w3.org/TR/css-box-3/)
- CSS Display Module Level 3 — block/inline box generation, anonymous
  boxes (https://www.w3.org/TR/css-display-3/)
- CSS 2.1 Visual Formatting Model / normal flow
  (https://www.w3.org/TR/CSS2/visuren.html,
  https://www.w3.org/TR/CSS2/box.html)
- Structural reference (approach only, not copied): Servo's layout crate

**Requirements:**
1. **Extend the existing cascade** (from v3) to also resolve, per element:
   `display` (`block` | `inline` | `none`), `background-color`, `margin`,
   `padding`, `border-width`, `border-color`. Reuse the same
   selector-matching/specificity/source-order machinery already built —
   don't build a parallel system.
2. **These new properties do not inherit** (unlike `color`): each
   element's value is either explicitly set or its initial/default, never
   taken from its parent.
3. **Default `display` per tag**, since there's no full user-agent
   stylesheet — a small hardcoded table: `html, body, div, p, h1–h6, ul,
   ol, li, blockquote, pre, header, footer, section, article, nav, aside,
   figure, figcaption, table, form, hr` default to `block`; everything
   else defaults to `inline`. Author CSS can override this via the
   `display` property.
4. **`display: none`** means no box at all — the element and its whole
   subtree are skipped in layout and painting, not just invisible.
5. **Lengths:** only `px` and unitless `0` are supported for
   margin/padding/border-width. Percentages, `em`, `rem`, and other units
   are out of scope.
6. **No explicit sizing:** `width`/`height` properties are out of scope —
   every block box takes the full available content width of its
   containing block (normal `width: auto` behavior).
7. **Layout tree:** build a tree of boxes from the DOM. A block-level
   child starts a new box; consecutive inline-level children (including
   text) directly inside a box are grouped into one anonymous inline run,
   laid out as wrapped colored text (reusing the existing cosmic-text
   rich-text path from v3).
8. **Block layout (normal flow only):** child boxes stack vertically,
   each full container width. No margin collapsing — margins are summed
   naively, not collapsed per spec. No floats, no positioning
   (`relative`/`absolute`/`fixed`), no flexbox/grid.
9. **Painting:** for each box, paint `background-color` (if set) across
   its border box, then its border (if `border-width` > 0 — uniform
   solid rectangle strokes on all four sides, no per-side styling, no
   border-radius), then its content (text, or recurse into block
   children).
10. **Streaming is preserved:** re-run layout and repaint from scratch on
    each stdin chunk, same "simplicity over incremental diffing" policy
    as v2/v3.
11. **No scrolling yet** — content taller than the window is still
    clipped; that's v5.

**Deliverable:** modifications to `scratchy`'s existing `main.rs`/
`Cargo.toml` implementing exactly the above.

---

## v5 — scrolling

**Requirements:**
1. Track a vertical scroll offset in pixels, clamped to
   `[0, max(0, total_content_height - window_height)]`.
2. **Input:** mouse wheel only (`WindowEvent::MouseWheel`, handling both
   winit's pixel-delta and line-delta variants). Keyboard scrolling
   (arrows/Page Up/Down/Home/End) is out of scope — a separate future
   feature.
3. **Painting:** shift every box's painted Y position by `-offset`;
   content above/below the viewport is clipped exactly as before.
4. **Re-clamp the offset** whenever the window resizes or content changes
   (new stdin chunks can change total content height).
5. **No scrollbar UI** — no visual indicator of scroll position. That's
   chrome, out of scope like the rest of the browser shell.

**Deliverable:** modifications to `scratchy`'s existing `main.rs`/
`Cargo.toml` implementing exactly the above.

---

## v6 — font-size, font-weight, font-style

Everything rendered so far uses one fixed font at one fixed size
(`cosmic-text`'s default sans-serif, 16px). This stage lets CSS vary
size, weight, and slant per element.

**Requirements:**
1. **Extend the existing cascade** (reuse the same selector-matching/
   specificity/source-order machinery from v3/v4 — no parallel system)
   to also resolve: `font-size`, `font-weight`, `font-style`.
2. **These three properties inherit**, unlike v4's box properties
   (`display`/`margin`/`padding`/`border-*`/`background-color`): an
   element with no winning declaration takes its parent's computed
   value, same inheritance model as `color` from v3. Root default:
   `font-size: 16px`, `font-weight: normal`, `font-style: normal` —
   matching today's hardcoded values, so a page with no font CSS at
   all renders identically to before.
3. **`font-size`:** only `px` is supported, same restriction as
   margin/padding/border-width in v4. `em`/`rem`/`%`/unitless are out
   of scope (separate future unit-support feature).
4. **`font-weight`:** keywords `normal` (400) and `bold` (700), plus
   raw numeric values (`100`–`900`) passed straight through to
   `fontdb::Weight`. No `bolder`/`lighter` relative keywords.
5. **`font-style`:** keywords `normal`, `italic`, `oblique`. No italic
   angle parameter for `oblique`.
6. **`font-family` is explicitly out of scope** for this stage — every
   element keeps rendering in `cosmic-text`'s default sans-serif
   regardless of weight/style/size. Family selection is a separate
   later feature.
7. **Line height follows font size per span:** each inline run's text
   already carries per-span color (v3); extend the same per-span
   `Attrs` to also carry size/weight/style via `Attrs::metrics` with a
   line height of `1.25 x font-size` (matching today's fixed 16px/20px
   ratio), so a line mixing sizes lays out with each run shaped at its
   own size rather than the whole paragraph at one size. Box/paragraph
   height derives from the shaped buffer's actual line heights, same
   mechanism already in place from v4/v5 — no separate calculation
   needed.
8. **Streaming is preserved:** re-run cascade/layout/repaint from
   scratch on each stdin chunk, same policy as every prior stage.

**Deliverable:** modifications to `scratchy`'s existing `main.rs`
implementing exactly the above.

---

## v7 — default heading size/weight

Found testing v6 against a real page
(`motherfuckingwebsite.com`, via `curl`): a page with zero CSS at all —
no `<style>`, no inline `style=`, nothing — renders every `h1`–`h3` at
the same 16px/normal weight as body text, because v6 only taught the
cascade *that* `font-size`/`font-weight`/`font-style` exist; it didn't
give headings any default. Real browsers do this via their user-agent
stylesheet; v4 already took the same approach for `display` (a small
hardcoded per-tag table, "since there's no full user-agent
stylesheet") — this does the same for heading size/weight.

**Requirements:**
1. **A hardcoded default (`font-size`, `font-weight`) per heading tag**,
   consulted exactly where v4's `default_display(tag)` is consulted —
   as the fallback *underneath* the cascade, not a special case above
   it: author CSS (including inherited `font-size` from a non-heading
   ancestor) still wins over this default the same way it already wins
   over `default_display`.
2. **Values** (matching typical browser UA defaults at a 16px root,
   since only `px` is supported — see v6 point 3): `h1` 32px/bold,
   `h2` 24px/bold, `h3` 19px/bold, `h4` 16px/bold, `h5` 13px/bold,
   `h6` 11px/bold.
3. **Every other tag is unaffected** — no default font-size/weight
   beyond the v6 root default (16px/normal), same as today.
4. **Does not inherit as a UA default** — a heading's *children* still
   inherit the heading's own *computed* size/weight normally (v6's
   existing inheritance), same as a `<b>` inside an `<h1>` today
   inherits the h1's resolved size. This table only supplies the
   heading element's own fallback value, exactly parallel to how
   `default_display` supplies one tag's own fallback `display`.

**Deliverable:** modifications to `scratchy`'s existing `main.rs`
implementing exactly the above.

---

## v8 — list markers

`ul`/`ol`/`li` already lay out as blocks (v4's `BLOCK_TAGS`), but with
no marker — a list reads as an unindented stack of paragraphs. This
adds the bullet/number itself.

**Requirements:**
1. **Each `<li>` gets a marker prefixed directly into its own text
   content**, as the first span of its first inline run: `"• "` (U+2022
   plus a space) when the `<li>`'s parent is anything other than `<ol>`
   (covers `<ul>` and malformed/missing-parent markup alike), or
   `"<n>. "` when the parent is `<ol>`.
2. **Numbering (`<ol>` only):** `<n>` is this `<li>`'s 1-based position
   among its parent's *direct* `<li>` children in document order — any
   non-`<li>` children of the `<ol>`/`<ul>` are skipped when counting,
   matching how browsers only number actual list items. No `<ol
   start>` or `<li value>` support — out of scope.
3. **Nested lists get independent counters for free:** since numbering
   only ever looks at one `<li>`'s immediate parent and that parent's
   direct `<li>` children, a nested `<ol>` inside a `<li>` counts from
   1 on its own regardless of depth — no explicit counter-stack needed.
4. **The marker uses the `<li>`'s own computed style** (color, and
   from v6/v7, size/weight/style) — same as any other text directly
   inside it, not a separately styleable pseudo-element.
5. **Explicitly out of scope:** indentation/hanging-indent (wrapped
   second lines of a long item do *not* align under the first line's
   text, only under the left edge like today — a separate future
   layout feature), `list-style-type`/`list-style-position`/`::marker`
   CSS, and default `ul`/`ol` margin/padding (still whatever v4's
   cascade already resolves them to, i.e. none unless authored).

**Deliverable:** modifications to `scratchy`'s existing `main.rs`
implementing exactly the above.

---

## v9 — default list margin/padding

v8 left default `ul`/`ol` margin/padding explicitly out of scope; the
WHATWG HTML Standard's suggested rendering (§15.3.7, "Lists") gives
`ol, ul { margin-block: 1em; padding-inline-start: 40px; }` as the
default UA styling real browsers use — same kind of gap v4
(`display`) and v7 (heading size/weight) already closed with a small
hardcoded per-tag table. This closes it for list indentation.

**Requirements:**
1. **A hardcoded default (`margin`, `padding`) for `ul`/`ol`**,
   consulted exactly where v7's `default_heading_font(tag)` is
   consulted — fallback underneath the cascade; author CSS (including
   an inherited `margin`/`padding` — though those don't inherit, per
   v4 — this only matters for an explicit author declaration on the
   element itself) still overrides it exactly as it already overrides
   `default_display`.
2. **Values:** `margin: 16px` (1em at the 16px root default) and
   `padding: 40px`, applied via scratchy's existing *single-scalar,
   all-sides* margin/padding model from v4 — not a new per-side box
   model (`margin-block`/`padding-inline-start` are each one-axis in
   the real spec; here they become a uniform margin/padding on all
   four sides, the same simplification v4's model already makes for
   every other element). No nested-list `margin-block: 0` override —
   out of scope, a nested list also gets the full 16px.
3. **`li` itself gets no default margin/padding** — matches the real
   spec (quoted above) exactly; only its `display: list-item` behavior
   (already `Display::Block` since v4's `BLOCK_TAGS`) and v8's marker
   are heading-table-free.
4. **Explicitly out of scope:** per-side margin/padding, `em`/`%`
   units (still `px`-only per v4/v6), nested-list margin collapsing,
   and `list-style-type`-driven marker shape by nesting depth (v8's
   bullet stays a plain disc at every depth).

**Deliverable:** modifications to `scratchy`'s existing `main.rs`
implementing exactly the above.

---

## v10 — default stylesheet as an embedded CSS file

v4, v7, and v9 each added a hardcoded Rust table standing in for a
piece of what a browser's user-agent stylesheet would otherwise
supply (`default_display`, `default_heading_font`,
`default_list_spacing`). This replaces all three with one actual CSS
file, parsed through the same cascade already built in v3, instead of
growing a fourth bespoke Rust table for the next one.

**Requirements:**
1. **A new `default.css`** (next to `src/main.rs`) containing exactly
   the rules the three tables it replaces already expressed:
   - `display: block` for the v4 `BLOCK_TAGS` list (one selector list,
     comma-separated).
   - `h1`–`h6` font-size/font-weight, same px values as v7.
   - `ul, ol` margin/padding, same px values as v9.
2. **Embedded at compile time** via `include_str!`, not read from the
   filesystem at runtime — no install-path/XDG-discovery problem for
   a single-binary tool, same reasoning discussed before building
   this. A missing/relocated file is a compile error, not a runtime
   one.
3. **Parsed once (`parse_stylesheet`, already built in v3) and
   prepended** to the document's own parsed rules — i.e. earlier in
   the same `Vec<StyleRule>` `resolve_style` already matches against,
   not a separate pass. The existing specificity/source-order cascade
   (v3) then does the override work for free: every rule in
   `default.css` is a plain element-type selector (the lowest possible
   specificity), so any author rule of equal or higher specificity
   already wins exactly as it does today; later-in-source-order ties
   favor whichever came later, which is always the document's own
   rule since it's appended after.
4. **`default_display`, `default_heading_font`, and
   `default_list_spacing` are deleted**, along with `BLOCK_TAGS` —
   their fallbacks become the properties' true CSS initial values
   (`display: inline`, `margin: 0`, `padding: 0`), matching today's
   behavior for every element `default.css` doesn't mention.
5. **No behavior change** — this is a pure refactor. The same pages
   that render correctly today (including the heading-size and list-
   indentation fixes from v7/v9) must render pixel-identically after.
6. **Marker text (v8) stays in Rust** — it's structural content
   injection, not a style property this cascade resolves, so there's
   nothing to move for it.

**Deliverable:** a new `scratchy/default.css` plus modifications to
`scratchy`'s existing `main.rs` implementing exactly the above.

---

## v11 — default.css rewritten against the real spec

v10 embedded `default.css` but only carried forward the handful of
values v4/v7/v9's old Rust tables already had. This replaces its
*content* with a proper pass against the actual UA-default rules:
fetch the WHATWG HTML Standard's "Rendering" chapter in full, cross-
check against Firefox's real `html.css` (reference only, not copied
in — MPL-2.0, full of `-moz-*` syntax and forms/media/ruby/frameset
rules scratchy has no use for), and hand-curate the overlap that's
actually relevant.

**Requirements:**
1. **Use the real property/selector names even where currently
   inert** (logical properties — `margin-block`, `margin-inline`,
   `padding-inline-start`, `margin-inline-start` — plus `display:
   table`/`table-cell`/etc., `text-decoration`, `font-family`,
   `list-style-type`), rather than inventing scalar workarounds for
   properties scratchy's box model can't yet express correctly. An
   unrecognized property name is already safely parsed-and-discarded
   (v3) and a selector that fails to parse already safely drops just
   that one rule (verified by hand, not assumed) — so this sits inert
   until a future feature implements it, then activates for free with
   no `default.css` changes needed.
2. **Three deliberate departures from the literal spec/Firefox text**,
   where following it verbatim would do nothing useful or actively
   break something that already works:
   - `font-size` stays `px` (the only unit scratchy parses) on
     headings, not the real `em`-relative values.
   - `li` stays `display: block`, not the real `list-item` (which
     would be inert today and break the block layout `li` currently
     gets, paired with v8's own marker-text injection).
   - `b`/`strong` get `font-weight: bold`, not the real relative
     `bolder` (a keyword scratchy's parser doesn't accept, which would
     leave `<strong>` not bold at all today).
3. **Covers, at minimum:** `body` margin; the full block-level tag
   list; headings' font-size/weight/margin; `p`/`blockquote`/`figure`/
   `pre`-family margins; lists (`ul`/`ol`/`dl`/`dd`/`li`) display,
   margin, padding, and nested-list margin/marker-shape overrides;
   `hr`; common inline semantics (`b`/`strong`/`i`/`em`/`cite`/`dfn`/
   `var`/`u`/`ins`/`s`/`del`/`strike`/`big`/`small`/`sub`/`sup`/`mark`/
   `abbr[title]`); `a` (styled directly on the element, not via
   `:link`/`:visited` — scratchy has no navigation at all, so there's
   no visited-state to distinguish); and `table`/`caption`/`col`/
   `colgroup`/`thead`/`tbody`/`tfoot`/`tr`/`td`/`th`.
4. **Known, accepted consequence:** replacing `ul`/`ol`'s v9/v10
   scalar `margin: 16px; padding: 24px` compromise with the real
   (inert) logical properties means lists go back to *zero* visible
   spacing today, until per-side margin/padding exists. This is the
   intended tradeoff, not a regression to chase — see `TODO.txt`.
5. **A new `TODO.txt`** tracking gaps surfaced along the way (inline
   `background-color` not reaching text spans — found via `mark`
   doing nothing; the single-scalar box model; missing units/
   properties) as a running, append-as-you-go checklist, separate from
   `PROMPTS.md` (append-only prompt log) and `NOTES.md` (free-form
   narrative).

**Deliverable:** a rewritten `scratchy/default.css` plus a new
`scratchy/TODO.txt`.

---

## v12 — inline background-color

`TODO.txt`'s top item: `default.css`'s `mark { background-color:
yellow; }` did nothing — `resolve_style()` computes
`background_color` for every element, but it's discarded once
recursion enters inline content, since `SpanStyle` (what a text run
actually carries into shaping) never had a background field at all.

**Requirements:**
1. **`SpanStyle` gains `background_color: Option<TextColor>`**,
   alongside its existing color/font_size/font_weight/font_style
   fields.
2. **Not CSS-inherited, but threaded like it is for painting
   purposes:** background-color doesn't inherit per spec (each
   element paints only its own box), but for an inline element split
   across a text run there's no separate "box" to paint it onto, so
   the nearest enclosing inline element's background is carried
   down to its descendant text/spans — overridden by a nested
   element's own background-color if it sets one, falling through
   unchanged otherwise (e.g. `<mark>outer <b>bold</b></mark>`: `b`
   has no background of its own, so it keeps showing `mark`'s). This
   is threaded as its own parameter through `collect_children`,
   separate from `InheritedStyle` (which stays true CSS inheritance),
   updated to the child's own `background_color` (if set) or else the
   value passed in, whenever recursing into an inline child.
3. **Per-span backgrounds painted as rects behind the glyphs**, not
   folded into a single box-model rect — a span can wrap across
   multiple lines, each needing its own rect clipped to that line.
   Implemented via cosmic-text's per-`Attrs` `metadata: usize` field
   (set to the span's index during `shape_spans`), read back per
   glyph from `buffer.layout_runs()` to merge each line's run of
   consecutive same-span glyphs into one rect (`glyph.x`/`glyph.w`
   give the span's pixel extent on that line, `run.line_top`/
   `run.line_height` the vertical extent), painted before the glyphs
   themselves so text stays legible on top.
4. **Shape cache carries the per-span background table alongside the
   buffer** (`ShapeCache`'s value becomes `(Buffer, f32,
   Vec<Option<TextColor>>)`), so paint can look backgrounds up by the
   same span index baked into each glyph's metadata without
   re-deriving them from the (already-consumed) `LayoutBox` tree.

**Deliverable:** modifications to `scratchy`'s existing `main.rs`
implementing exactly the above; removes the now-resolved item from
`TODO.txt`.

---

## v13 — per-side box model

`TODO.txt`'s next item, picked without further prompting as part of
working the list top-to-bottom: the box model's known architectural
limit (865ba44, d16e491) — `margin`/`padding`/`border-width` were each
a single scalar applied to all four sides, so none of `default.css`'s
real per-side/logical declarations (`margin-block`, `padding-inline-
start`, `margin-inline-start`, etc.) could do anything.

**Requirements:**
1. **A `Sides { top, right, bottom, left }` struct**, physical not
   logical — scratchy only ever lays out horizontal-tb/LTR, so block-
   start/end and inline-start/end collapse onto top/bottom and left/
   right with no separate representation needed. `ComputedStyle`'s
   `margin`/`padding` fields become `Sides`; `border_width` stays a
   single scalar (nothing in `default.css` sets a per-side border, so
   there's no real use for per-side border yet — not adding it until
   something needs it).
2. **Logical and shorthand property names expand to physical sides at
   parse time**, in `DeclParser`: `margin-top/right/bottom/left` and
   `padding-top/right/bottom/left` (physical longhands, net-new),
   `margin`/`padding` (existing shorthands, now writing all four
   physical fields instead of one scalar), `margin-block-start/end`
   and `margin-inline-start/end` (→ top/bottom and left/right
   respectively), `margin-block`/`margin-inline` (single-value
   shorthand form → both sides of that axis), and the same four
   logical forms for `padding`. Each cascades independently per
   physical side through the existing `cascade_property` machinery —
   so a more specific `margin-left` can still win over a less specific
   `margin` on the same element, same as real CSS.
3. **`auto` (e.g. `hr`'s `margin-inline: auto`) is not special-cased**
   — `parse_px_length` already rejects anything that isn't a px
   dimension or literal `0`, so `auto` simply drops the same way `em`
   does today: inert, not a crash, not a bogus value. True auto-margin
   centering (computing the box's own width against its container) is
   separate future work, not part of this.
4. **Layout (`measure`) and paint use the per-side values**: content
   box offsets use `margin.left`/`margin.top` and `padding.left`/
   `padding.top` on the leading edge, `border_w`/`content_w`/
   `border_h` subtract left+right or top+bottom independently instead
   of doubling one scalar, and sibling stacking in the block-children
   case adds the child's own `margin.top + margin.bottom` (not a
   doubled single value).
5. **Verified headlessly, not visually** — the sandbox's Wayland
   compositor stopped accepting screenshot requests partway through
   this session (`grim` fails with "failed to copy output eDP-1" even
   with no scratchy process involved at all, and scratchy's own
   `Resumed` winit event never fires either), so a `#[cfg(test)]`
   module was added temporarily, calling `resolve_style` directly on
   DOM nodes parsed from a small fixture and asserting the resulting
   `Sides` values land on the correct side with no cross-axis leakage
   (`ul`'s `padding-inline-start: 40px` only sets `padding.left`, not
   `.right` or `.top`; an inline `style="margin-left: 60px; margin-
   top: 4px"` only moves those two sides) — then removed before
   finishing, per this project's "every feature is preceded by the
   exact prompt that generated it" rule: a scratch verification harness
   isn't a feature of scratchy itself, nothing it adds persists.

**Deliverable:** modifications to `scratchy`'s existing `main.rs`
implementing exactly the above; updates `TODO.txt` (removes the
resolved item, updates the units item's wording now that per-side
property names parse and several of `default.css`'s px-based per-side
values are active).

---

## v14 — `em` length units

`TODO.txt`'s next item: v13 made per-side property *names* parse, but
left their *values* inert wherever `default.css` wrote them in `em`
(almost everywhere outside `body`/`hr`/`dd`/list `padding-inline-
start`, which happen to already be `px`). `font-size` itself was the
same story. Checked `default.css` directly before scoping this: it
uses only `px` and `em`, never `rem` or `%` anywhere — so only those
two are implemented, per this project's per-use-basis rule; `rem`/`%`
stay future work, same as before, just narrower.

**Requirements:**
1. **A `Length` enum (`Px(f32)` / `Em(f32)`)** replaces the raw `f32`
   in `Declarations` for every length-valued property (margin/padding
   per-side fields, `border-width`, `font-size`). Parsed once in
   `DeclParser` (`parse_length`, replacing `parse_px_length`); not yet
   resolved to a pixel value, since that requires knowing a font-size
   the parser doesn't have access to.
2. **Resolved at cascade time in `resolve_style`**, after the winning
   `Length` is already picked (so cascade/specificity logic is
   unchanged — `cascade_property` stays generic over `Length` the same
   way it was over `f32`): `Length::resolve(font_size)` multiplies an
   `Em` by the given `font_size`, passes a `Px` through unchanged.
3. **`font-size` resolves first, against the *parent's* font-size**
   (`inherited.font_size`) — moved up in `resolve_style` ahead of
   margin/padding/border-width, which resolve *second*, against this
   element's own just-resolved `font_size`. This matches real CSS: an
   element's `em` for box-model properties means "my own font-size,"
   but `font-size: Nem` on the element itself can only sensibly mean
   relative to what it's inheriting — there's no other value defined
   yet to be relative to.
4. **`rem`/`%`/unitless-non-zero stay unparsed**, same inert/drop
   behavior as before (and as `auto` already gets) — `parse_length`
   only recognizes `px`, `em`, and literal `0`.
5. **Verified headlessly** (`grim`/winit are still stalled in this
   sandbox — see v13, unrelated to this change, confirmed still
   failing standalone): a temporary `#[cfg(test)]` asserting `h1`'s
   `margin-block: .67em` resolves against `h1`'s own 32px font-size
   (not the inherited 16px root default), `p`'s `margin-block: 1em`
   resolves against the inherited 16px (not `h1`'s 32px), and `hr`'s
   plain-`px` `border-width`/still-inert `margin-inline: auto` are
   unaffected — then removed before finishing, same as v13.

**Deliverable:** modifications to `scratchy`'s existing `main.rs`
implementing exactly the above; updates `TODO.txt` (narrows the units
item to just `rem`/`%`/unitless, now that `px`/`em` are both done).

---

## v15 — table layout

`TODO.txt`'s next item, the largest/most speculative one on the list
(no real page had driven the need yet, unlike every prior item) — real
`display: table` layout. `default.css` already declares the full real
set of table-part `display` values (v11), all inert since none of
those keywords parsed and every table element fell back to the CSS
initial value (inline), silently flattening table markup into plain
text flow.

**Scoped deliberately smaller than full CSS auto table layout**, which
sizes each column from its cells' min/max content width — a
genuinely separate two-pass algorithm (shape once to find natural
width, size columns, reshape at final width). Implemented instead:
**equal-width columns** (closer to CSS's `table-layout: fixed`
behavior), no colspan/rowspan, no border-collapse/cell borders, no
caption-side placement — none of which `default.css` or any table
markup exercised so far actually need. Documented as a known
simplification in `TODO.txt`, not silently passed off as full table
support.

**Requirements:**
1. **New `Display` variants**: `Table`, `TableRow`, `TableCell`, and
   one `TableGroup` covering `table-row-group`/`table-header-group`/
   `table-footer-group`/`table-column-group`/`table-column` (`tbody`/
   `thead`/`tfoot`/`colgroup`/`col`) — all purely structural wrappers
   with no box of their own. `table-caption` maps to plain `Block`
   (renders in source position — no special placement logic, no
   `text-align` support to honor `caption`'s centering anyway).
2. **`display: table` is block-level** in the outer flow (extends the
   existing `Display::Block` check in `collect_children` to also
   match `Table`) but builds its content differently: `build_box_for_
   element` special-cases `Table` to call a new `build_table_children`
   instead of the normal block/inline flow builder.
3. **`build_table_children` walks a table's descendants for its rows**,
   transparently recursing through `TableGroup` wrappers (so `tr`s
   inside `thead`/`tbody`/`tfoot` are found the same as direct
   children), collecting plain `Block` content (e.g. `caption`) in
   place, and skipping anything else (stray cells/inline content
   outside a row isn't real-world markup). Does **not** recurse into
   a nested `Table` — that falls to the `Block` arm and gets its own
   independent box via the normal `build_box_for_element` call, which
   naturally re-enters table-building for its own rows.
4. **A new `BoxContent::TableRow(Vec<LayoutBox>)`** holds one row's
   cells. Rows themselves need no new box-content variant — they're
   ordinary `Blocks` children of the table (stacking vertically like
   any other block), only their *own* content (the cells) lays out
   horizontally instead.
5. **`measure` gains a `TableRow` arm**: splits the row's content
   width evenly across its cells, measures each cell at that width
   (reusing the ordinary block/inline measure path — a cell's own
   interior content works completely unchanged), and the row's height
   is the max over its cells' own margin+height. `paint` treats
   `TableRow` the same as `Blocks` (just walk and paint each child) —
   factored as one shared match arm via an `|` pattern, not a
   duplicated paint call.
6. **Verified headlessly** (`grim`/winit still stalled — see v13/v14,
   confirmed still failing standalone, unrelated to this change): a
   temporary `#[cfg(test)]` running a fixture through the *full*
   pipeline (`build_layout_tree` + `measure`, not just `resolve_style`
   this time) with a `caption`/`thead`/`tbody` table and a second,
   deliberately pathological table-nested-inside-a-cell-of-another-
   table — asserting rows stack vertically, cells split their row's
   width evenly, `thead`/`tbody` grouping doesn't change the row count
   found, and the nested table's row isn't incorrectly flattened into
   the outer table's row list — then removed before finishing, same
   as v13/v14.

**Deliverable:** modifications to `scratchy`'s existing `main.rs`
implementing exactly the above; updates `TODO.txt` (replaces the
general "no layout at all" item with the narrower equal-width-columns/
no-colspan/no-border-collapse simplifications actually shipped).

---

## v16 — `font-family`

`TODO.txt`'s next item: every element rendered in whatever generic
sans-serif `new_font_system()` resolved to (v6's `fc-match` fix),
regardless of `default.css`'s `tt, code, kbd, samp, pre { font-family:
monospace; }`. Checked `default.css` first, same as v14: it only ever
uses the bare `monospace` keyword, never a named family or a
comma-separated fallback list, so only the 5 CSS generic keywords are
implemented — no named-family/fallback-list parsing, a scope boundary
recorded in `TODO.txt` rather than silently implied as full support.

**Requirements:**
1. **A `FontFamily` enum** (`SansSerif`/`Serif`/`Monospace`/`Cursive`/
   `Fantasy`, `SansSerif` as `#[default]`) mirroring the 5 CSS generic
   keywords — matches cosmic-text's own `Attrs::new()` default, so
   every element without a `font-family` rule renders exactly as
   before. `to_cosmic()` maps it onto cosmic-text's own `Family` enum
   (re-imported as `CosmicFamily` to avoid clashing with ours).
2. **`font-family` is CSS-inherited** (same family as `color`/
   `font-size`/`font-weight`/`font-style`), so it's threaded through
   `InheritedStyle`/`ROOT_STYLE` the same way those are — simpler than
   v13's background-color, which genuinely isn't inherited and needed
   its own separate threading.
3. **Parsed as a single bare ident only** — a quoted family name's
   string token fails `expect_ident`, so e.g. `"Georgia", serif` drops
   the *whole* declaration (no fallback to `serif`), same inert-on-
   unsupported-value behavior as every other not-yet-supported CSS
   value in this codebase.
4. **Threaded through to `shape_spans`**: `SpanStyle` gains
   `font_family`, set on each span's `Attrs::family` right alongside
   the existing color/weight/style attrs.
5. **Verified past just the cascade** — same diligence as the v6
   `fc-match` bug (a field can cascade correctly and still not affect
   anything if the shaping path never reads it): a temporary
   `#[cfg(test)]` asserted `code`'s `font_family` resolves to
   `Monospace` via `resolve_style` *and* that shaping a sans-serif
   span and a monospace span side by side actually resolves to two
   different `fontdb::ID`s on this system, not just two different
   enum values that happen to render identically. `grim`/winit are
   still stalled in this sandbox (v13–v15), unrelated to this change
   — removed before finishing, same as every prior item this session.

**Deliverable:** modifications to `scratchy`'s existing `main.rs`
implementing exactly the above; updates `TODO.txt` (narrows the
font-family item to the named-family/fallback-list gap that remains).

---

## v17 — `text-decoration`

`TODO.txt`'s next item: `u`/`ins` (underline), `s`/`strike`/`del`
(line-through), and `a` (underline) declared `text-decoration` in
`default.css` since v11, all invisible. Turned out to need almost no
new code: cosmic-text's own `Attrs` already carries a `text_decoration:
TextDecoration` field (`underline`/`strikethrough`/`overline`, each
with an optional color override), and `Buffer::draw` — already called
by `paint_buffer` — already calls `render_decoration` internally using
real font underline/strikethrough metrics and falling back to each
glyph's own color. The only real work was parsing the CSS value and
propagating it the same way v12's background-color does.

**Requirements:**
1. **Parse `text-decoration` down to cosmic-text's own `TextDecoration`
   type directly** — no separate enum of this project's own, unlike
   `FontFamily` (v16): cosmic-text's type already *is* the right
   representation, re-exported from the crate root. `parse_text_
   decoration` loops the value's tokens (the shorthand can carry
   multiple, e.g. `default.css`'s `abbr[title] { text-decoration:
   dotted underline }`), recognizing `underline`/`line-through`/
   `overline`/`none`; anything else (a style keyword like `dotted`, or
   a color) is accepted without erroring but not tracked — every line
   drawn is solid, in the text's own color. Only fails (and drops,
   same convention as every other unsupported value) if *no* line
   keyword was found at all.
2. **Threaded as ambient propagation, same mechanism as v12's
   background-color** — `text-decoration-line` isn't a CSS-inherited
   property, but a UA draws an ancestor's decoration through
   descendant inline boxes that don't override it (e.g. `<a>link
   <b>bold</b></a>` — the `<b>` has no rule of its own but should still
   underline). `ComputedStyle.text_decoration` stays `Option
   <TextDecoration>` (cascaded, `None` = nothing declared on *this*
   element) and `collect_children` grows a `decoration: TextDecoration`
   parameter (concrete, not `Option` — a `TextDecoration` with
   everything off already represents "no decoration," which is exactly
   what a descendant needs to explicitly turn an ancestor's decoration
   back off via `text-decoration: none`).
3. **`shape_spans` sets `attrs.text_decoration = span.text_decoration`**
   per span — that's the entire painting side; `paint_buffer`'s
   existing `buffer.draw` call needed no changes at all.
4. **Verified past the cascade, through to cosmic-text's own shaped
   output** (same diligence as v6/v16 — confirming a field actually
   affects rendering, not just that it cascades correctly): a
   temporary `#[cfg(test)]` ran the full pipeline on an `<a>link
   <b>bold inside link</b> <span style="text-decoration: none">opted
   out</span></a>` fixture and inspected the shaped `Buffer`'s own
   `LayoutRun::decorations` (cosmic-text's computed decoration spans,
   not scratchy's own data) — confirming at least one span exists
   (propagation reached the `<b>`) and that none of them cover the
   "opted out" text's glyph range. `grim`/winit remain stalled in this
   sandbox (v13-v16) — removed before finishing, same as every prior
   item.

**Deliverable:** modifications to `scratchy`'s existing `main.rs`
implementing exactly the above; updates `TODO.txt` (replaces the
"not parsed/painted at all" item with the narrower style-keyword/
color gap that remains).

---

## v18 — `white-space: pre`

`TODO.txt`'s next item: text extraction (`normalize_whitespace`)
always collapsed whitespace, so `pre`/`listing`/`xmp`/`plaintext`
(`default.css`'s only `white-space: pre` rule) lost their literal line
breaks and spacing regardless of the CSS. Only `normal` (the existing
default) and `pre` are implemented — `default.css` never declares
`pre-wrap`/`pre-line`/`nowrap`, so neither is this item's job to add.

**Requirements:**
1. **`white_space` is genuinely CSS-inherited**, threaded through
   `InheritedStyle`/`ROOT_STYLE`/`ComputedStyle` the same simple way
   `font-family` (v16) is — no ambient-propagation complexity like
   background-color/text-decoration needed, since this one really does
   inherit normally.
2. **Text extraction branches on `inherited.white_space`** in
   `collect_children`'s text-node case: `Pre` pushes the node's raw
   text unchanged (no `normalize_whitespace`, no fabricated trailing
   space — byte-exact, newlines and multi-space runs intact); `Normal`
   keeps the existing collapse-and-append-a-trailing-space behavior
   unchanged.
3. **Wrapping is also disabled for `pre` content** — real CSS
   `white-space: pre` means both "don't collapse" and "don't wrap"; a
   `pre: bool` parameter on `shape_spans` calls `buffer.set_wrap(Wrap
   ::None)` when true, decided by `measure`'s `BoxContent::Inline` arm
   from the box's own `ComputedStyle.white_space` (not per-span — a
   `<pre>`'s content is always its own anonymous inline box, so
   box-level is the right granularity, and avoided adding yet another
   field to `SpanStyle` for it). Literal `\n`s in the text still start
   new lines either way — that's cosmic-text's own buffer-line
   splitting, unrelated to wrap mode.
4. **Verified through the full pipeline**: a temporary `#[cfg(test)]`
   fed a `<pre>` with mixed indentation/multi-space runs/newlines
   through `build_layout_tree` + `measure` at a narrow (80px) width,
   inspecting the shaped `Buffer`'s own `lines` (cosmic-text's actual
   `BufferLine`s, not scratchy's intermediate text) to confirm exact
   text survived and that `layout_runs()` still reported exactly 3 runs
   (one per source line) despite the narrow width — confirming wrap
   was genuinely disabled, not just coincidentally not triggered.
   `grim`/winit remain stalled in this sandbox (v13-v17) — removed
   before finishing, same as every prior item.

**Deliverable:** modifications to `scratchy`'s existing `main.rs`
implementing exactly the above; removes the now-resolved item from
`TODO.txt`.

---

## v19 — `list-style-type`

`TODO.txt`'s next item: `list_marker()` (v8) was a hardcoded Rust
function — plain bullet unless the parent tag was literally `ol`,
never consulting CSS at all, despite `default.css` declaring real
disc/decimal/circle/square rules (including nesting-depth-based
circle/square overrides) since v11.

**Requirements:**
1. **A `ListStyleType` enum** (`Disc`/`Circle`/`Square`/`Decimal`,
   `Disc` as `#[default]`, matching the real CSS initial value) — just
   the 4 keywords `default.css` actually declares, no `none` and none
   of the many other counter styles (roman numerals, alpha, ...).
   Genuinely CSS-inherited, threaded through `InheritedStyle` the
   simple way (same as v16/v18).
2. **`list_marker` takes the `<li>`'s own cascaded `list_style_type`**
   instead of re-deriving anything from the parent tag — `Decimal`
   counts among `li` siblings (the existing counting logic, now
   triggered by the *resolved style*, not a hardcoded `tag == "ol"`
   check); `Disc`/`Circle`/`Square` return the matching marker
   character (`•`/`◦`/`▪`). This is a real behavior improvement, not
   just a refactor: a `<ul>` with an explicit `list-style-type:
   decimal` now actually counts, which the old tag-based check could
   never do.
3. **Found and fixed a real, separate, previously-invisible bug while
   verifying this**: `default.css`'s nesting-depth circle/square rules
   use `:is(ol, ul, menu, dir) :is(ul, menu, dir) { ... }` — and
   `:is()` was never actually matching anything, because `selectors`
   crate's `Parser::parse_is_and_where()` defaults to `false` and
   `OurSelectorParser` never overrode it, so every `:is(...)` selector
   in `default.css` silently failed to *parse*, dropping the whole
   rule (same as any other unparseable selector) — including v9's
   "nested lists have no top/bottom margin" rule, which uses the same
   mechanism and has *also* been silently inert this whole time (only
   becoming a real, visible bug once v14 made `em` margins active).
   One-line fix: override `parse_is_and_where` to return `true`.
4. **Verified through the full pipeline, ancestor chain and all**: a
   temporary `#[cfg(test)]` fixture nested an `ol` inside a `ul` inside
   a `ul`, resolving each `<li>` by threading `.inherited()` down the
   *real* ancestor chain from the document root (not a shortcut
   `resolve_style(li, rules, ROOT_STYLE)`, which was tried first and
   wrongly skipped every ancestor's own cascaded values — a bug in the
   test itself, not the code, caught by the test failing in a way that
   didn't match the code's own logic). Confirmed: `ol` counts
   correctly, top-level `ul` is disc, 2-deep-nested `ul` is circle
   (only once the `:is()` fix landed), a nested `ol` inside a `ul`
   stays decimal (its own rule's specificity should still win over the
   depth selector), and — as a direct consequence of the same fix —
   the nested-list zero-margin rule is now also actually active.
   `grim`/winit remain stalled (v13-v18) — removed before finishing.

**Deliverable:** modifications to `scratchy`'s existing `main.rs`
implementing exactly the above; removes the now-resolved item from
`TODO.txt`. No `TODO.txt` entry added for the `:is()`/nested-margin
fix — it's a bug fix restoring already-declared, already-intended
`default.css` behavior, not a new deferred gap.

---

## v20 — `criterion`/`cargo bench`

`TODO.txt`'s last remaining item: every past performance
investigation (scroll lag, the redraw-storm bug, the shaping-budget
freeze) was settled by hand-editing `main.rs` with temporary
`eprintln!`/`Instant` timing, then removing it before committing — a
real `cargo bench` setup was agreed worth doing (NOTES.md) but never
done.

**Requirements:**
1. **Split `src/main.rs` into `src/lib.rs` (the engine) + a thin
   `src/main.rs`** (`fn main() { scratchy::run(); }`) — unavoidable,
   not a style choice: a `benches/*.rs` file is its own compilation
   unit and cannot reach into a binary crate's `main.rs` at all, only
   into a library crate it depends on. `fn main()`'s former body
   became `pub fn run()` in `lib.rs`, verbatim.
2. **Minimal `pub` surface**, not a blanket sweep — only what
   `benches/layout.rs` actually calls: `parse_html` (a new small
   helper wrapping the existing `new_parser()`/`TendrilSink` plumbing,
   so the bench doesn't need those low-level imports itself),
   `build_layout_tree`, `measure`, `new_font_system`, plus the
   `LayoutBox`/`Laid`/`ShapeCache` types those return or take (their
   *fields* stay private — a bench only ever holds these opaquely and
   feeds them to `black_box`, never reads into them, so Rust's
   "private type in public interface" rule doesn't force their fields
   open too).
3. **`benches/layout.rs` benchmarks two separable cost centers**
   against the same synthetic-but-realistic fixture (200 paragraphs of
   mixed inline content + a 20-item list + a 10x5 table — scratchy has
   no network access and no bundler tool yet to pull in a real page,
   so this is generated in the bench file itself, not fetched):
   `build_layout_tree` alone (DOM → CSS cascade → box tree, no
   shaping — the cost center behind the v1-v5 box-tree-flattening bug)
   and `measure` with a **fresh, empty `ShapeCache` every iteration**
   (real usage caches shaped paragraphs across frames — reusing one
   cache across bench iterations would measure almost nothing after
   the first) at a generous 60s deadline (always a complete pass, not
   a partial budgeted one — the cost center behind the resumable-
   shaping fix).
4. **Verified the bench actually runs and produces real numbers**, not
   just that it compiles: `cargo bench --bench layout -- --quick` —
   ~2.9ms for `build_layout_tree`, ~24.5ms for `measure` on this
   fixture.
5. **Confirmed the lib/bin split didn't regress the normal binary**:
   `cargo build` + launching `scratchy` against a small test file and
   checking its process state (sleeping, not crashed) — the same
   degree of verification available given `grim`/winit remain stalled
   in this sandbox (v13-v19).

**Deliverable:** `src/lib.rs` (new, the former `src/main.rs`'s
content, `pub fn run()` plus the minimal `pub` surface above),
`src/main.rs` (now a 3-line wrapper), `benches/layout.rs` (new),
`Cargo.toml` (`criterion` dev-dependency + `[[bench]]` entry); removes
the now-resolved item from `TODO.txt` — nothing left on it.

---

## v21 — `border` shorthand

TODO.txt (post-v20 feature-gap survey): `border: 2px solid #884400`
silently dropped the whole declaration — only the `border-width`/
`border-color` longhands were parsed (v13). User asked to start
picking features off that survey one at a time, one commit+push each,
skipping and noting anything that needs clarification.

**Requirements:**
1. **`parse_border_shorthand`**: loops tokens trying, at each position,
   a length (`border-width`), a recognized `border-style` keyword
   (`none`/`hidden`/`dotted`/`dashed`/`solid`/`double`/`groove`/
   `ridge`/`inset`/`outset` — consumed and discarded, since nothing
   reads border-style anywhere border-width/border-color are used:
   every border always paints as a plain solid stroke), or a color —
   in any order, each 0-1 times, same `<border-width> || <border-style>
   || <border-color>` grammar as the real shorthand. An unrecognized
   token is skipped rather than failing the whole value (same lenient
   philosophy as `parse_text_decoration` — one bad component shouldn't
   throw away a width/color that did parse).
2. **Wired into `DeclParser`** under the `"border"` property name,
   setting `border_width`/`border_color` in `Declarations` exactly
   like the longhand properties do — so a later longhand declaration
   in the cascade still overrides it, same as real CSS shorthand/
   longhand interaction.
3. **Verified headlessly**: width+color in either order, color-only
   (doesn't invent a 0 width... it already defaults to 0, more
   precisely: doesn't set a nonzero width), width-only, width+unknown-
   style-keyword (style keyword doesn't block the width from still
   parsing), and a later explicit `border-width` longhand overriding
   an earlier `border` shorthand.

**Deliverable:** `src/lib.rs` (`parse_border_shorthand` + one new
branch in `DeclParser::parse_value`); removes the resolved item from
`TODO.txt`.

---

## v22 — `text-align`

TODO.txt (post-v20 feature-gap survey, continued from v21): `caption {
text-align: center }` in `default.css` has always parsed but done
nothing — nothing read `text-align` anywhere.

**Requirements:**
1. **`TextAlign` enum** (`Left`/`Right`/`Center`/`Justify`, `Left`
   `#[default]` — matches the real CSS initial value for left-to-right
   content), parsed from the 4 keywords `text-align` accepts (no
   `start`/`end` — those are writing-mode-relative and scratchy has no
   notion of writing mode).
2. **Real CSS inheritance**, not the ambient-propagation pattern
   `background-color`/`text-decoration` use — `text-align` genuinely
   is a spec-inherited property, so it's threaded through
   `InheritedStyle`/`ComputedStyle`/`ROOT_STYLE`/`anonymous_style`
   exactly like `white-space`/`list-style-type` already are.
3. **No hand-rolled line-positioning logic** — cosmic-text already has
   this: `Buffer::set_rich_text`'s last parameter is
   `Option<cosmic_text::Align>`, previously always passed as `None`.
   `TextAlign::to_cosmic()` maps to it (`Left` stays `None` rather than
   `Some(Align::Left)`, so the common case takes the exact code path
   it always has); `shape_spans` gains an `align` parameter threaded
   straight through from the box's own `ComputedStyle.text_align`.
   Per-span background rects (v12) need no change — they're derived
   from each glyph's actual laid-out `x`/`w` after shaping, which
   already reflects whatever alignment shift cosmic-text applied.
4. **Verified headlessly**: cascade + inheritance (a `div` with
   `text-align: center` resolves to `Center`; a plain child `span`
   resolves to `Center` too, inherited, when resolved against the
   div's `.inherited()`), and — more load-bearing — that the alignment
   parameter actually moves glyphs: shaped `"hi"` in a 500px-wide
   buffer with `None`/`Some(Center)`/`Some(Right)` landed the first
   glyph at x=0/243.8/487.5 respectively, confirming the wiring (not
   just the cascade) works end to end.

**Deliverable:** `src/lib.rs` (`TextAlign` enum + `to_cosmic()`,
`Declarations`/`InheritedStyle`/`ComputedStyle`/`ROOT_STYLE`/
`anonymous_style` field + cascade, `text-align` branch in
`DeclParser::parse_value`, `shape_spans`'s new `align` parameter);
`default.css`'s existing `caption { text-align: center }` now actually
takes effect, no `default.css` change needed. Narrows the `TODO.txt`
vertical-align/text-align item down to just `vertical-align`.

---

## v23 — `vertical-align` (`sub`/`super`)

TODO.txt (post-v20 feature-gap survey, continued from v21/v22):
`default.css`'s `sub { vertical-align: sub }`/`sup { vertical-align:
super }` had always parsed but done nothing. Only `sub`/`super` are
implemented — the 2 keywords `default.css` actually declares — not
`top`/`middle`/`bottom`/`text-top`/`text-bottom`/length/percentage,
and not real per-table-cell vertical content positioning (`tr`/`td`/
`th`'s `vertical-align: inherit` has no `inherit` keyword to parse
anywhere in this codebase, so it just drops — harmless here, since
nothing above a `td` ever sets a non-`Baseline` value for it to
meaningfully inherit anyway).

**Requirements:**
1. **`VerticalAlign` enum** (`Baseline`/`Sub`/`Super`, `Baseline`
   `#[default]` — the real CSS initial value), *not* threaded through
   `InheritedStyle` — `vertical-align` genuinely isn't a spec-inherited
   property (unlike `text-align`, v22). Propagates the same **ambient**
   way `background-color`/`text-decoration` already do: a new `valign`
   parameter through `collect_children`, carrying an element's own
   non-`Baseline` value down to descendant spans that don't override
   it (`<sub>text <b>bold</b></sub>` — the `b` has no vertical-align of
   its own, but its span should still render shifted).
2. **No API in cosmic-text for this** (unlike `text-align`'s
   `Buffer::set_rich_text` alignment parameter — checked first, same
   as v22, but nothing analogous exists for baseline shifting).
   `Buffer::draw()`'s convenience wrapper offers no hook to nudge an
   individual glyph's y position either. Replaced it in `paint_buffer`
   with `buffer.draw()`'s own internals, copied out by hand from
   cosmic-text's public API (`LegacyRenderer`, `render_decoration`,
   `LayoutGlyph::physical`, all `pub`) — manually looping
   `buffer.layout_runs()`, looking up each glyph's per-span
   `VerticalAlign` by `glyph.metadata` (same per-span index
   `paint_span_backgrounds`, v12, already reads by), and adding a
   `y_shift` (±a fraction of that glyph's own font size — not real
   font-metric `subscriptYOffset`/`superscriptYOffset` values, which
   `fontdb`/cosmic-text don't expose; the same rough-fraction fallback
   browsers themselves use when a font doesn't supply its own) to
   `run.line_y` before computing the glyph's physical position.
3. **`shape_spans`/`ShapeCache`/`paint`/`paint_buffer` all gain a
   parallel `Vec<VerticalAlign>` alongside the existing `backgrounds`
   vector** (same per-span-index shape), threaded the same way.
   `VerticalAlign` had to become `pub` (was going to stay private) —
   `ShapeCache` is already `pub` (v20) and a public type alias can't
   reference a private one; variants come along as `pub` too, same
   unavoidable consequence `LayoutBox`/`Laid` already accepted in v20.
4. **Verified headlessly**: cascade resolves `sub { vertical-align:
   sub }` correctly *without* a sibling picking it up (confirms it's
   genuinely not CSS-inherited), a full tree build shows a nested `<b>`
   inside `<sub>` still carries `Sub` on its span (confirms the ambient
   propagation), `y_shift`'s sign (down for `Sub`, up for `Super`), and
   — the one that actually matters — rendering the same glyph through
   `paint_buffer` at `Baseline` vs. `Sub` into real pixel buffers and
   confirming the rasterized glyph's topmost ink row really is lower
   for `Sub` (not just that the layout math says it should be).
   Spot-checked visually too: `E = mc<sup>2</sup>` and `H<sub>2</sub>O`
   both render with the digit visibly raised/lowered.

**Deliverable:** `src/lib.rs` (`VerticalAlign` enum + `y_shift()`,
`Declarations`/`ComputedStyle`/`SpanStyle` field + cascade,
`vertical-align` branch in `DeclParser::parse_value`,
`collect_children`'s new `valign` parameter, `shape_spans`/
`ShapeCache`/`paint_buffer`'s new `Vec<VerticalAlign>` plumbing,
`paint_buffer`'s manual glyph-rendering loop replacing `buffer.draw()`).
Resolves the last item from the text-align/vertical-align `TODO.txt`
pair — nothing of the original pairing left.

---

## v24 — `line-height`

TODO.txt (post-v20 feature-gap survey, continued): `line-height` was
in the "misc CSS properties" cluster item. Picked out on its own since
it's a well-scoped, meaningful property with a single clear-cut
subtlety worth getting right, unlike several of its cluster-mates.

**Requirements:**
1. **`LineHeight` enum** (`Normal`/`Multiplier(f32)`/`Len(Length)`),
   parsed from `line-height`'s actual grammar — a bare `<number>`
   (unlike every other length-ish property here, not restricted to
   literal `0`; this is the property's single most common real-world
   form), `normal`, or a `px`/`em` length.
2. **Kept symbolic through `InheritedStyle`/`ComputedStyle`**, not
   resolved to a pixel value at cascade time like every other
   length-ish property here — this is the one property where that
   distinction is observable: a `<number>` (or `normal`) re-resolves
   against *each descendant's own* font-size, not the declaring
   element's. Caught by writing the test for it: `body { line-height:
   1.5 }` must give `h1` (default.css: `font-size: 32px`) a line-height
   of `48px` (`1.5 × 32`), not `24px` (`1.5 × 16`, body's own
   font-size) naively flattened down through inheritance — the latter
   would visibly squish every heading. An absolute `px`/`em` length,
   by contrast, genuinely is a flat inherited pixel value once
   resolved (against the *declaring* element's font-size, same timing
   as `margin`/`padding`/`border-width`'s own `em` resolution) and
   correctly does *not* rescale for a differently-sized descendant —
   both behaviors verified directly.
3. **Resolved to an actual `f32` only in `SpanStyle::from`**
   (`s.line_height.resolve(s.font_size)`), the one place a concrete
   number is needed — `cosmic_text::Metrics`'s second field, replacing
   the previously-hardcoded `font_size * 1.25` (which `LineHeight::
   Normal` — the default — still resolves to exactly, so a page that
   never declares `line-height` renders identically to before this
   feature).
4. **Verified headlessly**: the `Normal` default matches the old
   hardcoded `font_size * 1.25` exactly (no-declaration regression
   check), the multiplier-rescales-per-descendant case above, the
   absolute-length-doesn't-rescale case, and that `shape_spans`'s
   reported buffer height actually reflects a per-span `line_height`
   (100.0 in, 100.0 out) rather than the old constant. Spot-checked
   visually too: a `line-height: 2.5` paragraph next to a default one,
   wrapped to several lines each — the generous paragraph's line
   spacing is unmistakably wider.

**Deliverable:** `src/lib.rs` (`LineHeight` enum + `resolve()`,
`parse_line_height`, `Declarations`/`InheritedStyle`/`ComputedStyle`/
`ROOT_STYLE`/`anonymous_style`/`SpanStyle` field + cascade,
`line-height` branch in `DeclParser::parse_value`, `shape_spans`'s
`Metrics::new` now uses `span.line_height`). Narrows the misc-CSS-
properties `TODO.txt` item by one.

---

## v25 — `visibility: hidden`

TODO.txt (post-v20 feature-gap survey, continued): another pick from
the misc-CSS-properties cluster — `visibility` was entirely unparsed.
Only `visible`/`hidden` — no `collapse` (table row/column-specific,
`default.css` never uses it).

**Requirements:**
1. **`Visibility` enum** (`Visible`/`Hidden`, `Visible` `#[default]`).
   Real CSS inheritance (threaded through `InheritedStyle`, like
   `text-align`/`line-height`), *not* the ambient-propagation pattern
   `vertical-align` uses — the defining trait of `visibility` (unlike
   `display: none`) is that a descendant can set `visibility: visible`
   to become visible again despite a hidden ancestor, which real
   inheritance already gives for free: each text node's `inherited:
   InheritedStyle` parameter in `collect_children` already reflects
   the correctly-cascaded value at that exact point in the tree, no
   extra propagation parameter needed (verified directly — a `<span
   style="visibility: visible">` nested inside a `visibility: hidden`
   `<div>` resolves to `Visible`, and the full tree's actual span
   carries that correctly).
2. **Reserves layout space, paints nothing** — `measure` is untouched
   (never reads `visibility` at all, so hidden content occupies
   exactly the same space it would visible), only `paint`/
   `paint_buffer`/`paint_span_backgrounds` change. A box's own
   background/border (`paint_box_decoration`) is skipped when `s.
   visibility == Hidden`, but child boxes still recurse normally (each
   carries its own independently-resolved `visibility`). Per-span text
   and backgrounds are skipped via a new `Vec<bool>` parallel to the
   existing `backgrounds`/`valigns` vectors (same `glyph.metadata`
   per-span-index lookup established in v12/v23), threaded through
   `shape_spans`/`ShapeCache`/`paint_buffer`/`paint_span_backgrounds`.
3. **One accepted narrow simplification, documented inline**: decoration
   lines (underline/strikethrough) aren't filtered by the new hidden
   check — `render_decoration` draws per *run*, not per-span, and
   nothing in `default.css` pairs `text-decoration` with `visibility:
   hidden` today, so a hidden span with an ancestor's decoration
   showing through could theoretically still draw its underline. Not
   worth the extra plumbing until something actually exercises it.
4. **Verified headlessly**: cascade correctness including the
   descendant-overrides-back-to-visible case (both at the
   `resolve_style` level and the full-tree real-span level), and —
   painting a real glyph into pixel buffers — that hidden text paints
   literally nothing while reporting the *same* shaped height as the
   same text visible (confirms layout is untouched, only paint is).
   Spot-checked visually too: hidden inline text leaves its space
   reserved (visible gap) while being completely invisible.

**Deliverable:** `src/lib.rs` (`Visibility` enum,
`Declarations`/`InheritedStyle`/`ComputedStyle`/`ROOT_STYLE`/
`anonymous_style`/`SpanStyle` field + cascade, `visibility` branch in
`DeclParser::parse_value`, new `Vec<bool>` threaded through
`shape_spans`/`ShapeCache`/`paint`/`paint_buffer`/
`paint_span_backgrounds`, `paint`'s box-decoration skip). Narrows the
misc-CSS-properties `TODO.txt` item by one more.

---

## v26 — `outline`

TODO.txt (post-v20 feature-gap survey, continued): another pick from
the misc-CSS-properties cluster. `outline-width`/`outline-color`/the
`outline` shorthand were entirely unparsed.

**Requirements:**
1. **Mirrors `border-width`/`border-color` exactly** (same `Option
   <Length>`/`Option<CssColor>` `Declarations` fields, same
   `cascade_property` call shape, same `f32`/`Option<TextColor>`
   `ComputedStyle` fields, not inherited) — `outline` really is
   structurally identical to `border` as a CSS property, just painted
   in a different place with no box-model effect.
2. **Shorthand parsing shares its implementation with `border`'s**
   (v21): factored `parse_border_shorthand`'s loop out into
   `parse_border_like_shorthand(input, style_keywords)`, with
   `parse_border_shorthand`/`parse_outline_shorthand` as thin wrappers
   supplying their own (slightly different — `outline-style` allows
   `auto`, has no `hidden`) style-keyword lists. `outline-style` itself
   still isn't tracked, same as `border-style`.
3. **Drawn outside the border box, not part of the box model** — doesn't
   affect `w`/`h` the way `border_width` does (unlike border, which
   `resolve_style` folds into the content-box math). `paint_box_decoration`
   gained a second 4-rect ring, identical in shape to the border ring,
   offset *outward* by `outline_width` instead of inward. No
   `outline-offset` gap support (nothing needs one).
4. **Verified headlessly**: shorthand and longhand cascade correctly, a
   style-only `outline: dashed` doesn't invent a width, and — the two
   properties that actually matter for a "doesn't affect layout, does
   affect paint" feature — a real `measure()` call confirms an
   outlined box has the exact same width/height/position as the same
   box without an outline, while `paint_box_decoration` confirms pixels
   actually land in the ring strictly outside the border box (and
   nothing paints there without an outline declared). Spot-checked
   visually too: background + border + a thick outline together, outline
   cleanly outside the border with no layout shift.

**Deliverable:** `src/lib.rs` (`parse_border_like_shorthand` (factored
out of `parse_border_shorthand`) + `parse_outline_shorthand`,
`Declarations`/`ComputedStyle` fields + cascade, `outline-width`/
`outline-color`/`outline` branches in `DeclParser::parse_value`,
`paint_box_decoration`'s new outline ring). Narrows the
misc-CSS-properties `TODO.txt` item by one more.

---

## v27 — `box-shadow`

TODO.txt (post-v20 feature-gap survey, continued, following v26's own
note that this was the obvious next candidate): `box-shadow` was
entirely unparsed. Deliberately narrower than the real property —
each simplification is its own `TODO.txt` sub-item now, not folded
back into the misc-cluster one, since `box-shadow` is now implemented,
just not completely.

**Requirements:**
1. **`BoxShadow`/`ResolvedBoxShadow`**: `<offset-x> <offset-y>
   <blur-radius>? <spread-radius>? <color>` parsed via a dedicated
   token-loop parser (`parse_box_shadow`) — a `<color>` is accepted at
   any position (real CSS allows it first or last), but unlike the
   shorthand parsers (v21/v26), an unrecognized token drops the whole
   declaration rather than being skipped — safer than guessing at
   which length means what with noise mixed in.
2. **`inset` is actively rejected, not silently misrendered**: an
   inner shadow needs an entirely different paint (clipped to the
   box's own content area, not drawn outside it) — rendering it as an
   outer shadow would look wrong, not just incomplete, so
   `parse_box_shadow` returns `Err` the moment it sees the `inset`
   keyword, same "drop the whole declaration" outcome as any other
   unparseable value.
3. **No blur, no `currentColor` fallback, single shadow only** — all
   three documented inline and in `TODO.txt` as their own narrower
   items, not blanket "not implemented": `blur-radius` is still parsed
   (so its presence doesn't trip the reject-on-unrecognized-token
   path) but discarded; a missing `color` fails the whole parse rather
   than falling back to the element's own text color.
4. **Resolved like `outline`** — `Declarations.box_shadow: Option
   <BoxShadow>` (unresolved lengths/color) cascades to `ComputedStyle.
   box_shadow: Option<ResolvedBoxShadow>` (resolved pixels + paintable
   `TextColor`) in `resolve_style`, not inherited.
5. **Painted first in `paint_box_decoration`**, behind this box's own
   background/border — the correct stacking for a non-inset shadow —
   as a single filled rect at `(x + offset_x - spread, y + offset_y -
   spread)`, sized `(w + 2·spread, h + 2·spread)`.
6. **Verified headlessly**: basic offset+color parsing, blur-radius
   discarded while spread-radius is still captured, color-first
   ordering, `inset` correctly rejects the whole declaration (not
   misrendered), a missing color correctly rejects too, a real
   `measure()` call confirms a shadowed box has identical
   width/height to the same box without one (shadow is paint-only,
   same proof style as v26's outline test), and that the shadow
   actually paints a rect at its offset position behind the box.
   Spot-checked visually too: a hard-edged dark offset shadow at the
   bottom-right of a background+padding box.

**Deliverable:** `src/lib.rs` (`BoxShadow`/`ResolvedBoxShadow` structs,
`parse_box_shadow`, `Declarations`/`ComputedStyle` fields + cascade,
`box-shadow` branch in `DeclParser::parse_value`,
`paint_box_decoration`'s new shadow rect). Removes `box-shadow` from
the misc-CSS-properties `TODO.txt` cluster, replaced with its own
narrower-scope item.

---

## v28 — `border-radius` (background-only)

TODO.txt (post-v20 feature-gap survey, continued — after letter-
spacing was attempted and reverted, and opacity was scoped out as
needing more design input than this run could resolve alone).
`border-radius` was entirely unparsed.

**Requirements:**
1. **`fill_rounded_rect`**: a new row-by-row rounded-rect fill
   primitive alongside the existing `fill_rect`, same hard-edged (no
   anti-aliasing) fidelity level as every other shape this file
   paints — not a visual downgrade. Each row is a single horizontal
   span, inset within `radius` of the top/bottom edge by how far that
   row sits from the nearest corner's circular boundary.
2. **Deliberately scoped to the background fill only** — combined
   with a visible border (`border_width > 0`), `border-radius` is
   ignored entirely and both fall back to the existing square-corner
   code path, unchanged. A correctly-rounded *ring* needs real
   per-pixel inner/outer-boundary membership testing: the "paint a
   bigger solid shape, then a smaller one on top" trick this file
   otherwise favors (and that would have made a rounded border trivial
   to express as two nested `fill_rounded_rect` calls) doesn't work
   once the interior isn't guaranteed opaque — there's nothing to
   safely "restore" where a hollow (no-background) ring should show
   through. Not worth building the real ring-math without a page that
   actually needs it.
3. **Not inherited** (`ComputedStyle`-only field, same shape as
   `outline_width`/`box_shadow` — not threaded through
   `InheritedStyle`).
4. **Verified headlessly**: cascade resolves correctly and doesn't
   inherit to a child, `fill_rounded_rect` itself actually clips a
   corner pixel that a plain `fill_rect` would paint (while still
   painting dead center, confirming it's a real shape change, not a
   no-op), `paint_box_decoration` rounds the background when there's
   no border, and falls back to painting the corner square when a
   border is present. Spot-checked visually too: a background-only box
   with visibly rounded corners next to a background+border box that
   correctly stays square.

**Deliverable:** `src/lib.rs` (`fill_rounded_rect`,
`Declarations`/`ComputedStyle` fields + cascade, `border-radius`
branch in `DeclParser::parse_value`, `paint_box_decoration`'s
background-fill branch). Removes `border-radius` from the
misc-CSS-properties `TODO.txt` cluster, replaced with its own
narrower-scope item.
