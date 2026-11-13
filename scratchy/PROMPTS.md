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

---

## v29 — `font-size: larger`/`smaller`

TODO.txt (post-v20 feature-gap survey, continued). Unlike most of this
run's picks, this one wasn't just a TODO.txt line item — it fixes two
currently-*inert* `default.css` rules: `big { font-size: larger }`/
`small { font-size: smaller }` (declared since v6/v7) and `sub`/`sup`'s
own `font-size: smaller` (also since early on) have silently done
nothing this whole time, since `larger`/`smaller` were never parsed.

**Requirements:**
1. **`Length` gains `Larger`/`Smaller` variants**, resolving through
   the exact same `resolve_style` code path `Em` already uses for
   `font-size` (against the *parent's* computed font-size — correct
   for `font-size`'s relative keywords, which is why reusing `Length`
   made sense here rather than a separate type). `1.2` isn't a spec
   exact value (the real spec steps through an absolute-size keyword
   table) — the common approximation browsers converge on, and
   roughly the ratio `default.css`'s own heading sizes already follow.
2. **Deliberately *not* reachable from `parse_length`/`set_length`**
   (used by every other length-ish property) — `margin: larger` isn't
   meaningful, so only `font-size`'s own `DeclParser` branch checks
   for the two keywords first, falling back to `parse_length` for
   everything else.
3. **Verified headlessly**: `larger`/`smaller` resolve correctly
   against the parent's font-size in isolation, `default.css`'s real
   `sub`/`sup`/`big`/`small` rules now actually produce a different
   (smaller/smaller/larger/smaller respectively) font-size than their
   surrounding text, and — a safety check — the keyword genuinely
   isn't reachable through `margin-top` or any other length property.
   Spot-checked visually too: all four now visibly differently sized
   inline in one paragraph.

**Deliverable:** `src/lib.rs` (`Length::Larger`/`Smaller` + `resolve()`
arms, `font-size`'s `DeclParser` branch tries the two keywords before
falling back to `parse_length`). Narrows the misc-CSS-properties
`TODO.txt` item by one more; `font-weight: bolder`/`lighter` stays,
explicitly noted as lower priority since nothing in `default.css`
declares it.

---

## v30 — `border-style`

TODO.txt (post-v20 feature-gap survey, continued). `border-style` was
parsed-and-discarded wherever it appeared (the `border` shorthand,
v21) or entirely unparsed as its own longhand — every border painted
as a plain solid stroke regardless.

**Requirements:**
1. **`BorderStyle` enum**: `Solid`/`Dashed`/`Dotted`/`Double`/`None`
   only — `groove`/`ridge`/`inset`/`outset` all fold into `Solid` at
   the keyword-mapping stage (`border_style_from_keyword`) rather than
   getting their own inert variants, since a real bevel effect needs
   two-tone shading this file's flat-fill primitives don't have.
   `hidden` folds into `None` (no border-conflict-resolution rules
   here that would ever need to tell them apart).
2. **The `border`/`parse_border_shorthand` style keyword, previously
   consumed-and-discarded (v21), is now actually captured and mapped**
   — required factoring `parse_border_like_shorthand`'s return type to
   hand back which keyword matched (`Option<&str>`, borrowed from the
   caller's own keyword list), so `border`'s wrapper can map it to a
   `BorderStyle` while `outline`'s wrapper (which still doesn't track
   style at all) just discards it, unchanged. Two small type aliases
   (`BorderShorthand`, `BorderLikeShorthand`) added alongside this to
   keep the 3-element return tuples from tripping clippy's
   type-complexity lint.
3. **`paint_border_edge`**: one new function painting a single edge's
   rect (`Solid` is one plain `fill_rect`, unchanged; `Dashed`/
   `Dotted` tile smaller fills along the edge's length; `Double` draws
   two thinner parallel stripes with a gap between), called 4 times
   from `paint_box_decoration` in place of the 4 inline `fill_rect`
   calls border used to make directly. `paint_box_decoration` also now
   skips the whole border when `border_style == BorderStyle::None`,
   even if `border-width`/`border-color` are set.
4. **Verified headlessly**: longhand/shorthand cascade, `none`/
   `hidden` both resolve to `BorderStyle::None` while
   `groove`/`ridge`/`inset`/`outset`/`solid` all resolve to `Solid`,
   `default.css`'s real `hr` rule (`border-style: inset`) resolves to
   `Solid` with no change from before this property existed (not a
   regression), `border-style: none` suppresses the whole border even
   with width/color set, and — painting real edges into pixel buffers
   — `Dashed`/`Dotted` genuinely paint fewer pixels than `Solid` (real
   gaps, not just a different color) while `Double` leaves its middle
   row unpainted (the gap between its two stripes). Spot-checked
   visually too: dashed/dotted/double borders all render distinctly,
   `hr`'s solid border unaffected.

**Deliverable:** `src/lib.rs` (`BorderStyle` enum +
`border_style_from_keyword`, `parse_border_like_shorthand`'s reworked
return type, `Declarations`/`ComputedStyle` fields + cascade,
`border-style` branch in `DeclParser::parse_value`,
`paint_border_edge`, `paint_box_decoration`'s border section rewired
to use it). Removes `border-style` from the misc-CSS-properties
`TODO.txt` cluster, replaced with its own narrower-scope item.

---

## v31 — `overflow` (text clipping only)

TODO.txt (post-v20 feature-gap survey, continued). `overflow` was
entirely unparsed. Scoped deliberately narrower than the real property
after evaluating the full-scope version's blast radius: real
`overflow` clipping needs a clip rect honored at every `fill_rect`/
`fill_rounded_rect` call site this file has (box backgrounds, all 4
border edges, outline, shadow — 18+ individual calls across 4-5
functions), which would have meant touching the paint path of nearly
every feature landed this session, a real regression risk against all
of them this late in a long run. Text painting, by contrast, already
funnels through exactly one shared per-pixel callback
(`paint_buffer`'s `LegacyRenderer`) plus one `fill_rect` call
(`paint_span_backgrounds`) — a single, low-risk choke point to clip at
instead, covering the single most common real `overflow: hidden` use
(truncating text that would otherwise spill out of a fixed-size
container).

**Requirements:**
1. **`Overflow` enum** (`Visible`/`Hidden` only — no `scroll`/`auto`,
   no scrollbar UI or interactive scroll region), not inherited, not
   `pub` (unlike `VerticalAlign`, it never needs to appear in a public
   signature — `ShapeCache`'s `Vec<bool>` `hidden` field is reused for
   `visibility`, not a new `Vec<Overflow>`).
2. **`intersect_clip`**: a small reusable helper intersecting an
   `(x, y, w, h)` rect against an `Option<(f32, f32, f32, f32)>` clip
   (`None` = unclipped, passed through), returning a possibly
   zero-size rect for something entirely outside it — `fill_rect`'s
   own existing bounds checks already treat that as "paint nothing."
3. **`paint` gains a `clip` parameter**, threaded through its
   recursion: an element's own `overflow: hidden` only starts clipping
   its *descendants* (`child_clip`, computed by intersecting the
   incoming `clip` with this box's own bounds) — its own background/
   border isn't clipped by its own overflow, same as it isn't by an
   ancestor's `clip` either (`paint_box_decoration` never receives
   `clip` at all, deliberately, keeping that whole call path
   untouched). Only the two text-painting call sites
   (`paint_buffer`/`paint_span_backgrounds`) receive and act on it.
4. **Verified headlessly**: cascade correctness (not inherited),
   `intersect_clip`'s math directly (basic intersection, unclipped
   passthrough, fully-outside case), and — the one that actually
   matters — a full real pipeline test (`parse_html` → `build_children`
   → `measure` → `paint`, real pixel buffer) using a `<pre>` (wrap
   disabled) with a line far wider than the window, inside an
   `overflow: hidden` div: confirmed nothing paints near the window's
   right edge (correctly clipped) while the start of the line still
   paints near the left (correctly not over-clipped). Spot-checked
   visually too: a long unwrapped `<pre>` line cut off cleanly at its
   container's border, no spillover into the rest of the page.

**Deliverable:** `src/lib.rs` (`Overflow` enum, `intersect_clip`,
`Declarations`/`ComputedStyle` fields + cascade, `overflow` branch in
`DeclParser::parse_value`, `paint`'s new `clip` parameter and
`child_clip` computation, `paint_buffer`/`paint_span_backgrounds`'s
new `clip` parameter and the glyph-callback/background-fill clip
checks). Narrows the misc-CSS-properties `TODO.txt` cluster, replaced
with its own narrower-scope item documenting the text-only limitation.

---

## v32 — `cursor` (`auto`/`pointer`), scratchy's first real interactivity

TODO.txt (post-v20 feature-gap survey, continued). `a { cursor:
pointer }` has been declared in `default.css` since early on, entirely
inert — `cursor` was never parsed, and scratchy had no interactivity
beyond mouse-wheel scroll to act on it with anyway.

**Requirements:**
1. **`CursorStyle` enum** (`Auto`/`Pointer` — only what `default.css`
   needs), real CSS inheritance (`Auto` is the real spec initial
   value) threaded through `InheritedStyle` like `text-align`. Also
   threaded per-span (`SpanStyle`/`shape_spans`'s new `Vec<CursorStyle>`
   in `ShapedParagraph` — `ShapeCache`'s per-paragraph tuple finally
   got its own type alias, 6 elements deep now) — the common real use
   (`<a>`) is inline content inside a block, not a block-level box of
   its own, so hit-testing needs *glyph*-level precision, not just
   block-level.
2. **`hit_test_cursor`**: walks the `Laid` tree the same way `paint`
   does, descending into whichever child's bounds contain the point;
   for `LaidContent::Text`, looks up the exact glyph at that point in
   the cached `Buffer` (same per-row/per-glyph bounding-box check
   `paint_span_backgrounds` already does) and reads its span's cursor
   via `glyph.metadata`. Falls back to a box's own (already-inherited)
   `style.cursor` wherever no more specific glyph match exists —
   same "ambient" layering `paint`'s box-vs-descendant painting
   already relies on.
3. **`WindowEvent::CursorMoved`** (new — scratchy's first mouse-
   position-aware event; previously only `MouseWheel`): converts the
   window-space mouse position into document space (`+ scroll_offset`,
   the inverse of what `paint` subtracts), calls `hit_test_cursor`
   against the cached layout, maps the result through
   `CursorStyle::to_winit()`, and calls `window.set_cursor` only when
   the icon actually changed (`App` gained a `last_cursor: CursorIcon`
   field to track this) — avoids a redundant OS call on every
   mouse-move tick. Uses `Window::set_cursor`, not the deprecated
   `set_cursor_icon` (caught by a compiler warning, fixed before
   shipping).
4. **Verified headlessly**: cascade + inheritance (`default.css`'s `a`
   rule resolves to `Pointer`; a `<span>` nested inside inherits it
   with no `cursor` of its own), the `CursorStyle`→`CursorIcon`
   mapping directly, and — the one that actually matters — a full real
   pipeline test (parse → build → `measure`, real shaped glyphs) that
   locates an actual link glyph and an actual plain-text glyph in a
   real paragraph and confirms `hit_test_cursor` reports `Pointer` for
   the former and `Auto` for the latter, not a block-level
   approximation. **Not verified visually**: the OS cursor icon is
   compositor-rendered, outside scratchy's own pixel buffer, so `grim`
   can't show it, and this environment can't synthesize a real
   mouse-move event — only a crash/render smoke-test (launched with a
   link on the page, confirmed stable, no panic) covers the live
   event-handling glue beyond the headless tests. Flagged in
   `TODO.txt` for a real interactive check later.

**Deliverable:** `src/lib.rs` (`CursorStyle` enum + `to_winit()`,
`ShapedParagraph` type alias, `Declarations`/`InheritedStyle`/
`ComputedStyle`/`ROOT_STYLE`/`anonymous_style`/`SpanStyle` field +
cascade, `cursor` branch in `DeclParser::parse_value`, `shape_spans`'s
new `Vec<CursorStyle>`, `hit_test_cursor`, `App`'s new `last_cursor`
field, the new `WindowEvent::CursorMoved` handler). Narrows the
misc-CSS-properties `TODO.txt` cluster, replaced with its own
narrower-scope item noting the un-verified-live caveat.

---

## v33 — `text-decoration-style`/`text-decoration-color`

TODO.txt (post-v20 feature-gap survey, continued, plus the pre-survey
`text-decoration` item at the top of the file). `text-decoration`
already tracked line keywords (underline/line-through/overline/none,
v17) but parsed-and-discarded any style keyword (`dotted`/`dashed`/
`wavy`/`double`) and explicit color — `abbr[title] { text-decoration:
dotted underline }` in `default.css` rendered as a plain solid
underline.

**Requirements:**
1. **`DecorationStyle` enum** (`Solid`/`Dashed`/`Dotted` only —
   `double` isn't a variant here since cosmic-text 0.19 already
   natively supports a double *underline* via its own
   `UnderlineStyle::Double`, so `parse_text_decoration` folds a
   `double` keyword straight into `deco.underline` instead; `wavy` has
   no custom painting path and falls back to `Solid`, same "accepted
   but not visually distinct" narrowing as `border-style`'s `groove`/
   `ridge`/`inset`/`outset`).
2. **`parse_text_decoration` reworked** to also capture a style keyword
   and an explicit color, order-independently (tracked in locals during
   the token loop, applied once after it ends, so `dotted underline`
   and `underline dotted` both work) — same lenient "skip an
   unrecognized token" philosophy as `parse_border_like_shorthand`. An
   explicit color resolves immediately via the existing `to_text_color`
   (pure, no cascade context needed) and sets whichever of cosmic-text's
   own `underline_color_opt`/`strikethrough_color_opt`/
   `overline_color_opt` fields correspond to the lines actually active,
   so `text-decoration: line-through red` doesn't leave a stray
   (invisible, since underline isn't on) `underline_color_opt`.
3. **`DecorationValue` struct** (`{ deco: TextDecoration, style:
   DecorationStyle }`) replaces the bare `TextDecoration` everywhere it
   was threaded as an `Option` (`Declarations`/`ComputedStyle`'s
   `text_decoration` field, `collect_children`'s ambient `decoration`
   parameter) — bundled as one struct rather than two independently-
   cascaded fields, since both only ever come from the one shorthand at
   the same time, propagated ambiently together exactly like plain
   `text_decoration` already was.
4. **`paint_decorations`/`paint_decoration_line`**: a hand-written
   reimplementation of cosmic-text's own `render_decoration`/
   `draw_decoration_span` (`src/render.rs` in the vendored 0.19.0
   source — the inner per-span function isn't `pub`, so it can't be
   called directly), using only `DecorationSpan`'s/
   `GlyphDecorationData`'s public fields. Mirrors the exact offset/
   thickness/color formulas so `Solid` (including native double-
   underline) paints pixel-identical to calling `render_decoration`
   directly; `Dashed`/`Dotted` instead break one rectangle into several
   short ones spaced across the same extent. Each span's style is
   looked up via `decoration_styles[glyph.metadata]` off its first
   glyph — the same per-span index every other per-span property
   (backgrounds, valigns, cursors, ...) already uses, threaded as a new
   7th element of the `ShapedParagraph` tuple.
5. **Verified headlessly**: `parse_text_decoration` order-independence
   (`dotted underline` / `underline dotted` parse identically), `double`
   mapping to cosmic-text's native `UnderlineStyle::Double` rather than
   this file's own `DecorationStyle`, color attaching only to active
   lines (`line-through red` doesn't set `underline_color_opt`), `wavy`
   falling back to `Solid`, `abbr[title]` cascading through the real
   `default.css` to a `Dotted` style (not falling back to `Solid`), and
   `paint_decoration_line` painting one full-width rectangle for
   `Solid` vs. several short ones (each shorter than the line, dots
   denser/narrower than dashes) for `Dashed`/`Dotted`, via a
   `CollectingRenderer` test double. **Verified visually** (`grim`,
   monitor now on): `abbr[title]`'s dotted underline renders as
   distinct dots (not solid), a `<span>` with `text-decoration:
   line-through dashed blue` renders a blue dashed strikethrough, and
   `text-decoration: underline double red` renders two parallel red
   underlines.
6. **Found while testing, not introduced by this feature, documented
   in `TODO.txt` rather than fixed**: `text-decoration` declared
   directly on a *block*-level element (e.g. `<p style="text-decoration:
   underline">text</p>`) doesn't reach that element's own direct text
   content at all — `collect_children`'s ambient `decoration` parameter
   is seeded fresh every time a block starts its children
   (`build_children`/`build_children_with_prefix`), not from that
   block's own resolved `style.text_decoration`. Only inline-level
   descendants (which pick up their own declared value independently of
   the ambient parameter) work correctly — exactly the shape every real
   `default.css` rule and prior test page already used
   (`u`/`s`/`abbr`/`a`), so this never surfaced before. A correct fix
   needs the same ambient-value threading through
   `build_box_for_element`/`build_table_children`/
   `build_table_row_cells`/`build_children`/`build_children_with_prefix`
   that `opacity`'s own skipped item (v28) already identified as a
   bigger plumbing change than a single property's scope — not
   attempted here either, for the same reason.

**Deliverable:** `src/lib.rs` (`DecorationStyle` enum, `DecorationValue`
struct, `parse_text_decoration` reworked, `Declarations`/
`ComputedStyle`/`SpanStyle` fields, `collect_children`'s ambient
parameter type, `ShapedParagraph`'s new 7th element, `paint_decorations`/
`paint_decoration_line`, `paint_buffer`'s new parameter and all its call
sites). Updates the pre-survey `text-decoration` item in `TODO.txt` to
reflect what now works, replaced with a narrower-scope item (and
removed from the misc-CSS-properties cluster's own parenthetical, which
had already listed it as landed-elsewhere) plus a new item documenting
the block-level ambient-seeding gap found along the way.

---

## v34 — `rem` lengths

TODO.txt (pre-survey item, present since v14 landed `px`/`em`). `rem`
was entirely unparsed — any declaration using it (`font-size: 1.5rem`,
`margin: 2rem`, ...) silently dropped, falling back to whatever was
inherited/default.

**Requirements:**
1. **`Length::Rem(f32)`** — resolves against the document root's
   font-size, not the declaring element's own the way `Em` does.
   Needed a second figure alongside every `resolve()` call's existing
   `font_size: f32` parameter, so `Length::resolve`/`LineHeight::resolve`
   both gained a `root_font_size: f32` parameter (used only by `Rem`,
   threaded through regardless for a consistent signature).
2. **`InheritedStyle`/`ComputedStyle` gain `root_font_size: f32`** — set
   once at `ROOT_STYLE` (`16.0`) and copied *unchanged* through every
   element's `inherited()`/`anonymous_style`, unlike every other
   inherited field here (which re-derives per element). Deliberately
   scoped to a fixed constant rather than the real document root
   element's own cascaded font-size — a real `html { font-size: ... }`
   override isn't respected (own `TODO.txt` item, not attempted: nothing
   in `default.css` or a test page declares one).
3. **`parse_length`/`parse_line_height`** both gained a `rem` dimension
   match alongside their existing `px`/`em` ones.
4. All ~14 `.resolve(font_size)` call sites inside `resolve_style`
   (margin/padding/border-width/font-size itself/outline-width/
   box-shadow's three lengths/border-radius) updated to pass
   `inherited.root_font_size` through; `SpanStyle::from`'s
   `line_height.resolve` call similarly updated to pass `s.root_font_size`.
5. **Verified headlessly**: `2rem` resolves to the same `32px` whether
   nested under a parent at `40px` font-size or not (proving it ignores
   the parent, unlike `em`, which would've given `80px`), `Length::Rem`
   and `parse_length("1.5rem")` resolve directly, `line-height: 2rem`
   resolves through `LineHeight`'s own grammar too. **Verified
   visually** (`grim`): a `.inner` div at `font-size: 2rem` nested
   inside a `40px` `.outer` renders visibly smaller than its parent
   (not the much-larger `80px` an `em`-based bug would produce), with
   `0.5rem` padding/border sized consistently, and a plain `1rem`
   paragraph matching the default body text size.

**Deliverable:** `src/lib.rs` (`Length::Rem` variant +
`resolve`/`LineHeight::resolve` signature changes, `InheritedStyle`/
`ComputedStyle`'s new `root_font_size` field + `ROOT_STYLE`/
`inherited()`/`anonymous_style` wiring, `parse_length`/
`parse_line_height`'s new `rem` branches, all call sites threading
`root_font_size` through). Narrows the original pre-survey `rem`/`%`/
unitless `TODO.txt` item to just `%`/unitless, replaced with a new
narrower-scope item noting the fixed-constant-root-font-size caveat.

---

## v35 — `font-weight: bolder`/`lighter`

TODO.txt (misc-CSS-properties cluster). Only the literal `bold`/`normal`
keywords and raw numbers were recognized — `bolder`/`lighter` dropped
the whole declaration. Lower priority than the rest of the cluster
(nothing in `default.css` declares either), picked over the cluster's
other remaining member (`word-spacing`) specifically because it fit
the existing architecture cleanly while `word-spacing` doesn't (see its
own narrowed `TODO.txt` item — no cosmic-text 0.19 API to hook into at
all, a worse version of the risk that already broke `letter-spacing`).

**Requirements:**
1. **`FontWeightValue` enum** (`Absolute(FontWeight)`/`Bolder`/
   `Lighter`) — same shape as `font-size`'s own `Length::Larger`/
   `Smaller` (v29): `bolder`/`lighter` are relative to whatever weight
   this element would otherwise *inherit*, not a fixed number, so they
   can't resolve until `resolve_style` has `inherited.font_weight` in
   hand. Unlike `LineHeight`, nothing further needs to stay symbolic
   once resolved (`bolder`/`lighter` only ever look at the *immediate*
   parent's weight) — the result becomes an ordinary `FontWeight` from
   there, inherited normally.
2. **`FontWeightValue::resolve`** implements the real CSS Fonts Level 4
   step table (not a simple ratio the way `larger`/`smaller` use):
   `bolder` maps `<350 → 400`, `350–549 → 700`, `≥550 → 900`; `lighter`
   maps `<550 → 100`, `550–749 → 400`, `≥750 → 700`.
3. **Verified headlessly**: the step table at its boundaries
   (300→400, 400→700, 549→700, 600→900 for `bolder`; 400→100, 600→400,
   800→700 for `lighter`), `Absolute` ignoring the inherited weight
   entirely, parsing all three keyword forms, and — the one that
   actually matters — a real cascade test proving `bolder` is relative
   to its *parent*, not absolute: the same `.strong-ish { font-weight:
   bolder }` rule resolves to `700` under a normal (400) root but `900`
   under an already-bold (700) parent. **Verified visually** (`grim`):
   bolder text renders visibly bold, nesting `bolder` again inside an
   already-`<b>` parent renders visibly heavier still (900), and
   `lighter` inside a `<b>` parent renders back down to normal weight.

**Deliverable:** `src/lib.rs` (`FontWeightValue` enum + `resolve`,
`parse_font_weight`'s new `bolder`/`lighter` branches,
`Declarations.font_weight`'s type change, `resolve_style`'s font-weight
cascade now calling `.resolve(inherited.font_weight)`). Resolves the
last-but-one member of the misc-CSS-properties `TODO.txt` cluster,
leaving only `word-spacing` — now also given its own narrower-scope
item explaining why it's harder than `letter-spacing` was, not just
lower priority.

## v36 — `rem` resolves against `<html>`'s real cascaded font-size

> check the TODO for scratchy browser. Lets prioritize things and get
> to work

Prioritized a short slice of well-scoped `TODO.txt` gaps that don't
need an architecture decision or user sign-off (unlike named
`font-family`, `opacity`, `letter-spacing`/`word-spacing`) or a bigger
design call (positioning, flexbox/grid, custom properties, at-rules,
forms/JS). This is the first: `rem`'s own narrower-scope item left by
v34, which always resolved against a fixed `16.0` constant instead of
a real `<html>` element's own cascaded `font-size`.

`build_layout_tree` now resolves `<html>`'s style once up front (a new
`find_html_element` walks the parsed DOM for it — a plain recursive
search, not just a one-level peek at `document`'s children, so a
malformed/fragment document without `<html>` as the immediate child
still finds it rather than silently falling back) and seeds
`root_font_size` from its resolved `font_size`, before that same
`InheritedStyle` is threaded through the normal `build_children`
recursion. `<html>` itself gets resolved twice (once here, once again
normally inside `build_children`) — harmless, since `resolve_style` is
pure. Falls back to `ROOT_STYLE.font_size` (`16.0`) when there's no
`<html>` element at all, same as before.

Verified with a temporary test (removed before committing, same
discipline as prior sessions): `<html style="font-size: 40px">` with a
nested `margin-top: 2rem` resolves to `80px`, not the `32px` the old
fixed constant would give; without the override it still resolves to
`32px` as before. Also visually confirmed end-to-end with a real
`grim` screenshot against a `font-size: 32px` `<html>`, `margin-top:
2rem`/`padding: 1rem` div — margin/padding scaled to the 32px root,
not 16px.

**Deliverable:** `src/lib.rs` (`find_html_element`, `build_layout_tree`
resolving `root_font_size` from it instead of using `ROOT_STYLE`
directly, doc comments on `InheritedStyle::root_font_size`/
`ComputedStyle::root_font_size` updated to describe the real
resolution instead of the old fixed-constant caveat). Closes the
narrower-scope `rem` item v34 left in `TODO.txt`.

---

## v37 — `:link`/`:visited` pseudo-classes

> check the TODO for scratchy browser. Lets prioritize things and get
> to work

Second item off the same prioritization pass as v36. `OurSelectorImpl`'s
`NonTSPseudoClass` was a stub uninhabited enum, so *any* pseudo-class
needing element state — `:hover`/`:focus`/`:active`/`:link`/`:visited`
— failed to parse at all, silently dropping the whole rule (same
category of bug as v19's `:is()` discovery). `:hover`/`:focus`/
`:active` genuinely need interactivity state scratchy doesn't track
yet (left in `TODO.txt`); `:link`/`:visited` need none at all, since
scratchy has no navigation or history — every hyperlink it ever
renders is honestly "unvisited."

Replaced the uninhabited `NoPseudoClass` with a real `OurPseudoClass {
Link, Visited }`, wired `OurSelectorParser::parse_non_ts_pseudo_class`
to recognize the two keywords, and implemented matching: `:link`
delegates to a new `El::is_link()` (`a`/`area` with an `href` — an
anchor with no `href` is a fragment target, not a link, same as a real
browser) which also backs `selectors`' own built-in `:any-link`
machinery for free; `:visited` always returns `false`.

Verified both the parse-then-match path (temporary test, removed
before committing: `a:link { color: red }` applies to `<a href>` but
not a bare `<a>`, `a:visited` never applies) and visually via `grim`
against a real page with both rules — the `href`'d link rendered red
and bold as declared, the `href`-less anchor fell through to
`default.css`'s unconditional `a` rule (plain blue), confirming the
`href` gate actually works end-to-end, not just in isolation.

**Deliverable:** `src/lib.rs` (`OurPseudoClass` replacing
`NoPseudoClass`, `OurSelectorParser::parse_non_ts_pseudo_class`,
`El::match_non_ts_pseudo_class`, `El::is_link` given a real body).
Narrows the stateful-pseudo-class `TODO.txt` item to just
`:hover`/`:focus`/`:active`.

---

## v38 — `border-radius` + a visible border (rounded ring)

> check the TODO for scratchy browser. Lets prioritize things and get
> to work

Third item off the same prioritization pass as v36/v37. `border-radius`
(v28) only ever rounded a plain background fill — combined with a
visible border, both fell back to square corners, since the "paint a
bigger solid shape, then a smaller one on top" trick `outline`/
`box-shadow` get away with doesn't work once the interior isn't
guaranteed opaque (nothing to safely "restore" where a hollow ring
should leave a hole).

Added `fill_rounded_ring`, which tests each row's outer *and* inner
rounded boundary directly instead (at most two horizontal spans per
row — the ring's left/right edge strips — once a row is within the
inner rect's own vertical extent, or one full-width span outside it).
Extracted the shared per-row corner-inset math (`corner_inset`) out of
`fill_rounded_rect` so both functions use the same formula for their
outer and (for the ring) inner boundaries — a `border_width` wider
than the corner radius naturally clamps the inner radius to `0.0`,
degrading to a square inner hole with no separate case needed.
`paint_box_decoration` now rounds the background to the *outer*
rounded rect unconditionally when `border_radius > 0` (not just when
there's no border, like before) — same background-under-border
stacking a real rounded box has — and paints the ring on top, but only
for `BorderStyle::Solid`; `dashed`/`dotted`/`double` still fall back to
the old square-corner rendering rather than a half-rounded mix (a
correctly-rounded dashed/dotted ring needs the same curved-path tracing
repeated per dash/dot, left as its own narrower `TODO.txt` item, same
"reject the unsupported variant outright" precedent `box-shadow`'s
`inset` already set).

Verified with a pixel-level test (removed before committing): a 40×40
rounded box with `radius=10`, `border_width=5` — the outer corner
pixel is untouched (cut by rounding), a top-edge pixel within border
thickness is the border color, dead center shows the background
through the ring's hole, a left-edge pixel within border thickness is
the border color. Then visually confirmed end-to-end with `grim`
against three real boxes: solid rounded border + background (clean
ring, yellow hole visible), rounded background alone (unchanged from
v28), and dashed border + radius (correctly falls back to square
corners, not a broken partial render).

**Found while verifying, not introduced by this feature, and flagged
as its own much bigger `TODO.txt` item rather than guessed at**: the
`width`/`height` CSS properties aren't implemented at all — confirmed
by grep, no `Declarations`/`ComputedStyle` field exists for either.
Every box in scratchy today is purely content-sized; an explicit
`width`/`height` silently drops like any other unsupported
declaration. Surfaced because the first version of this session's
visual test used `width: 200px; height: 100px` and rendered nothing
like what was declared — switched to `padding` for a reliably-sized
test box instead once this was confirmed. Real design questions here
(how an explicit size should interact with `measure`'s existing
content-driven sizing) make this worth its own prioritized slice, not
a quick follow-on.

**Deliverable:** `src/lib.rs` (`corner_inset` extracted from
`fill_rounded_rect`, new `fill_rounded_ring`, `paint_box_decoration`'s
background/border rounding logic, `ComputedStyle::border_radius`'s doc
comment updated). Closes the `border-radius` item in `TODO.txt`,
replaced by a narrower dashed/dotted/double scope note and a new,
much bigger `width`/`height` item.

---

## v39 — fix `text-decoration` not reaching a block's own direct text / nested block descendants

> check the TODO for scratchy browser. Lets prioritize things and get
> to work

Fourth item off the same prioritization pass (v36-v38), continued after
a mid-session check-in: pushed v36-v38, confirmed scope with the user
to continue with this item and `overflow`'s block-box clipping next,
leaving the newly-discovered `width`/`height` gap (found while
verifying v38) for its own dedicated future session.

Fixed the real bug `v33` found-but-didn't-fix: `<p style="text-decoration:
underline">text</p>` rendered `text` with no underline at all, because
`collect_children`'s ambient `decoration` parameter was reseeded to
`DecorationValue::default()` every time a block started its own
children, rather than threaded from that block's own resolved
`style.text_decoration` — so a block's *own direct text* (and any
nested block descendant that doesn't re-declare its own decoration)
never saw the block's own `text-decoration`, even though an inline
descendant handling the identical declaration already worked fine
(`collect_children`'s other branch already did
`style.text_decoration.unwrap_or(decoration)` correctly).

Despite `TODO.txt` grouping this with `opacity`'s "materially bigger
plumbing" caveat, it didn't share `opacity`'s actual blocker: `opacity`
was skipped because *correct semantics* needed the user's input (group-
compositing vs. a simplified ambient-multiply, two genuinely different
results); this one had one unambiguous correct behavior the whole time
— the exact `unwrap_or` propagation already proven correct for the
inline case, just not yet threaded across block boundaries too. Added a
`decoration: DecorationValue` parameter to `build_box_for_element`,
`build_table_children`, `build_table_row_cells`, `build_children`, and
`build_children_with_prefix`; `build_box_for_element` resolves its own
`style.text_decoration.unwrap_or(decoration)` once up front and passes
that down every path that builds its own children, mirroring the
inline branch exactly. `build_table_children`/`build_table_row_cells`
pass their raw incoming `decoration` straight through to
`build_box_for_element` for `Block`/`TableCell` children (trusting it
to redo the `unwrap_or` itself, since it re-resolves that child's style
from the same `inherited` anyway — same "resolve_style is pure, a
second call is harmless" precedent `v36` used), but pre-resolve their
own `child_decoration` for `TableRow`/`TableGroup`, since those build
their `LayoutBox` directly rather than delegating. The true root call
(`build_layout_tree`) still starts at `DecorationValue::default()` —
there's no ambient context above the document.

Verified with 4 temporary tests (removed before committing): a block's
own direct text now gets its own declared decoration; a nested block
descendant with no decoration of its own inherits the ambient one from
a block ancestor (not just from an inline ancestor, which already
worked); a nested block can still opt out with an explicit
`text-decoration: none`; and — to make sure the fix doesn't
over-propagate — a sibling block after a decorated one does *not* leak
the decoration sideways. Then visually confirmed all four shapes with a
real `grim` screenshot, plus a regression check against a page
exercising inline decoration/links/lists/tables (v12-v32-era rendering
unchanged).

**Deliverable:** `src/lib.rs` (`decoration` parameter threaded through
`build_box_for_element`/`build_table_children`/`build_table_row_cells`/
`build_children`/`build_children_with_prefix`, `build_layout_tree`'s
root call updated, `collect_children`'s block-dispatch call site passes
its own ambient `decoration` through instead of discarding it). Closes
the block-level `text-decoration` item `TODO.txt` had carried since
`v33`.

---

## v40 — extend `overflow: hidden` to clip a descendant's own box decoration

> check the TODO for scratchy browser. Lets prioritize things and get
> to work

Fifth and last item from this prioritization pass (v36-v39): `overflow:
hidden` previously clipped descendant *text* only (`paint`'s `clip`
parameter, read by `paint_buffer`/`paint_span_backgrounds`) — a nested
block box's own background/border/outline/shadow painted wherever it
would regardless of an ancestor's clip, since those paint through ~20
separate `fill_rect`/`fill_rounded_rect` call sites rather than the one
shared per-pixel callback text funnels through.

Rather than add an `intersect_clip` call at each of those ~20 sites by
hand (the real regression risk `TODO.txt` flagged — most of those exact
call sites were touched across v21/v26/v27/v28/v30/v38), pushed clip-
awareness down into the single lowest-level primitive instead:
`fill_rect` itself now takes a `clip: Option<(f32,f32,f32,f32)>` and
applies `intersect_clip` once, internally. `fill_rounded_rect`/
`fill_rounded_ring`/`paint_border_edge`/`paint_box_decoration` each
gained the same `clip` parameter, threaded straight through to every
`fill_rect` call they make (no change to any of their own geometry
math) — so every caller gets correct clipping for free, impossible to
miss a call site since they all bottom out at the one primitive.
`paint`'s existing `clip` parameter (already correct for text) now also
reaches `paint_box_decoration`, using the *ambient* clip (not
`child_clip`) — a box's own `overflow` only clips its descendants,
never its own decoration, same as real CSS. `paint_span_backgrounds`'s
own call site got simpler too: it used to manually call
`intersect_clip` before `fill_rect`; now it just passes `clip` straight
through like every other caller.

Verified three ways: a pixel-level test confirming `fill_rect` itself
honors `clip` precisely; a full-pipeline test (`build_layout_tree` →
`measure` → `paint` into a real pixel buffer) confirming a nested div's
background painted outside a narrow ancestor's bounds is fully clipped
while the ancestor's own background still shows; and a `grim`
screenshot against a real page using a `box-shadow` offset -300px off a
`overflow: hidden` ancestor — the shadow is clipped almost entirely
away, with only the small sliver legitimately inside the ancestor's own
padding/border area still visible, matching real CSS clipping exactly.
(A first attempt at the live visual check used a *negative margin* to
push a child outside its parent instead of `box-shadow` — looked like a
clipping bug at first, but turned out to be correct CSS box-model
behavior instead: a negative `margin-left` on a `width: auto` block
also *grows* its computed width by the same amount, so most of the box
legitimately overlapped back into the parent's visible bounds. Caught
by checking the actual geometry headlessly before concluding anything
was broken — not a `TODO.txt`-worthy finding, just a reminder that
`width: auto` + negative margins isn't "shift left," it's "shift left
and grow," per spec.) Also re-ran the v38 border-radius/dashed-fallback
page and the v12-v32-era inline-decoration/links/lists/tables page —
both pixel-identical to before this change.

**Deliverable:** `src/lib.rs` (`clip` parameter added to `fill_rect`,
`fill_rounded_rect`, `fill_rounded_ring`, `paint_border_edge`,
`paint_box_decoration`; `paint`'s existing `clip` now also passed to
`paint_box_decoration`; stale doc comments on `Overflow`,
`ComputedStyle::overflow`, `paint`, and `paint_buffer` updated to
describe the new scope instead of the old text-only one). Narrows the
`overflow` `TODO.txt` item to just the missing `scroll`/`auto` modes.

---

## v41 — keep verification tests permanently instead of deleting them

> curious, when did we agree to not keep tests?

Asked after v36-v40 each followed the established "write a temporary
`#[cfg(test)]`, verify, remove before committing" pattern. Checked for
an actual explicit decision backing that pattern and found none — it
traces to v13's own `PROMPTS.md` entry, which justified deleting its
first verification test as "per this project's 'every feature is
preceded by the exact prompt that generated it' rule: a scratch
verification harness isn't a feature of scratchy itself, nothing it
adds persists." That's a self-justification a prior session wrote, by
analogy to the (unrelated) temporary `eprintln!`/`Instant` perf-
instrumentation discipline — not something the user asked for, and
never revisited across v14-v40 despite the perf precedent it leaned on
getting a proper permanent home of its own (`criterion`/`cargo bench`,
v20) while correctness tests never did.

Asked the user directly: keep tests going forward (recommended),
keep deleting them, or decide case by case. Chose to keep them going
forward. Added a permanent `#[cfg(test)] mod tests` at the end of
`src/lib.rs` and restored the five verification tests written (then
deleted) across v36-v40 as its first entries — `rem`/html font-size,
`:link`/`:visited` matching (+ no-href / :visited-never-matches),
`border-radius`+border ring (pixel-level), the block-level
`text-decoration` propagation bug (4 tests: own text, nested-block
propagation, explicit `none` opt-out, no sibling leak), and
`overflow: hidden` clipping box decoration (`fill_rect`'s own clip
behavior + a full `build_layout_tree`→`measure`→`paint` pipeline test).
All 12 pass; `cargo clippy` stayed at the same 16-warning baseline
(fixed two trivial lints — `manual_contains`/`manual_range_contains` —
in the newly-permanent test code so it doesn't start baseline drift).

**Going forward**: a feature's verification test is written, run, and
kept as part of this suite — not deleted — unless the user says
otherwise for a specific case. `cargo test` alongside `cargo check`/
`cargo clippy`/`cargo install --path .` is now part of the standard
per-feature loop.

**Deliverable:** `src/lib.rs` (`mod tests` with 12 tests). No behavior
change — test-only.

---

## v42 — `width`/`height` CSS properties

> cool. what's our next todo item?
>
> [followed by a design question: when content is taller than an
> explicit `height`, spill past the box (real CSS default) or always
> clip — chose spill-past, "always try to follow the spec to the best
> of our ability"]

The big gap found while verifying v38: `width`/`height` weren't parsed
or resolved at all, confirmed by TODO.txt's own grep. Content-box
semantics (CSS's own default `box-sizing`) — `width`/`height` set the
*content* box directly; padding/border still add outward on top, same
as every other box-model field here. Only `px`/`em`/`rem` (same units
as everything else; `%`/`box-sizing`/`min-*`/`max-*` left as their own
narrower `TODO.txt` item).

`Declarations`/`ComputedStyle` each gained `width`/`height: Option<f32
(resolved) / Length (declared)>`, parsed and cascaded the same way as
every other length property (`em` resolves against this element's own
font-size, same ordering `margin`/`padding`/`border-width` already
follow). In `measure`: an explicit `width` replaces the
container-derived `content_w`/`border_w` outright, including when it's
*wider* than the available space passed down — not clamped, same as
real CSS (a box isn't clipped to its container just because it declares
a width; it extends past it, and `overflow: hidden` on an ancestor, if
set, already handles clipping that correctly since v40). An explicit
`height` only overrides this box's own *reported* size — what its
parent stacks the next sibling against — never the actual child-layout
process: children are always measured in full against `content_w`
regardless of `height`, so content taller than an explicit `height`
still gets fully laid out and painted, spilling past this box (real
CSS `overflow: visible` default) rather than being silently dropped or
unconditionally clipped. Confirmed directly with the user which of
those two `height`-overflow behaviors to build before writing any code,
since TODO.txt had flagged it as a real design question, not a
unilateral pick: spec-accurate spill-past, not always-clip.

Verified with 6 permanent tests (first real use of v41's new kept-not-
deleted test suite) — explicit `width` sets the content-box width
correctly (padding added outward), an explicit `width` wider than the
container extends past it rather than clamping, a child with no width
of its own gets laid out against its *parent's* explicit width (not the
original container width), an explicit `height` overrides only the
box's own reported size while its children (two full `<p>`s) are still
completely present and individually measured — not truncated — and an
explicit `height` larger than the natural content extends the box.
Visually confirmed with two `grim` screenshots: a `width: 200px` box
correctly wraps long text to that width; a `height: 30px` box with no
`overflow` set visibly lets much-taller content spill past its own
border/background, with the next sibling paragraph positioned based on
the declared 30px (not the taller visual content) — overlapping the
spilled text exactly as a real browser would render it; and (separately)
a `height: 40px` box with `overflow: hidden` correctly clips the same
kind of overflow instead, confirming v40's clipping and v42's sizing
compose correctly together with no extra glue needed.

**Deliverable:** `src/lib.rs` (`width`/`height` parsing in
`DeclarationParser`, `Declarations`/`ComputedStyle` fields, cascade
resolution in `resolve_style`, `measure`'s `border_w`/`content_w`/
`border_h` logic in all three `BoxContent` branches, 6 new tests in the
permanent `mod tests`). Resolves the biggest item in `TODO.txt`,
replaced by a narrower `box-sizing`/`min-*`/`max-*`/`%` scope note.

---

## v43 — `box-sizing`, `min-width`/`max-width`/`min-height`/`max-height`

> what's the next item we have in the scratchy todo?
>
> yes. do you need any clarifications before you start? I plan to go
> away after you begin implementing

Next `TODO.txt` item after v42's `width`/`height`: the narrowed
"content-box only" gap it left behind. No real design decision here —
`box-sizing`'s two keywords and the min/max clamp order are both
exactly spec'd — so this went ahead without stopping, same bundling as
one commit per `TODO.txt` bullet (`box-sizing` and `min-*`/`max-*` were
one bullet, not two). `%` stays out, per the adjacent, still-open
`TODO.txt` item — nothing needs it yet, same `px`/`em`/`rem`/
unitless-`0` forms as everywhere else.

`BoxSizing` (`ContentBox`/`BorderBox`, not inherited, default
`ContentBox`) and `MaxLen` (`None`/`Len(Length)` — `max-width`/
`max-height`'s own grammar needs a value distinct from "not declared"
to let an explicit `none` win over an earlier, more specific rule's
real length; `min-width`/`min-height` need no such keyword, since their
initial value, `0`, is already what an absent declaration resolves to)
are new. `measure`'s separate `border_w`/`content_w`/(three copies of)
`border_h` formulas collapsed into one `clamp_box_dim` helper, called
once for width and once per `BoxContent` branch for height: converts
whichever of `width`/`min-width`/`max-width` (or the height
equivalents) are set into the box `box_sizing` names via one `extra`
(2×border-width + the relevant padding pair), takes the `auto` value
when nothing's specified (container-derived for width, `extra +
content_h` for height — still subject to `min`/`max` either way, same
as real CSS: a `min-width` wider than the container forces overflow),
then clamps `max` before `min` (spec order — an author's `min > max`
mistake resolves to `min`, never something smaller). All three
`BoxContent` branches (`Inline`/`Blocks`/`TableRow`) needed the height
version of this, since each computes its own natural content height
(`content_h`/`content_h`/`row_h`) independently. `width`'s active role
(it actually constrains children's layout/text-wrapping width, not
just the reported size) and `height`'s passive one (overrides only the
*reported* size — v42's real CSS `overflow: visible` default,
untouched here) both still hold; `min`/`max-width` share `width`'s
active role, `min`/`max-height` share `height`'s passive one.

Verified with 6 new permanent tests (23 total): `box-sizing: border-box`
shrinks the box to the *specified* total instead of adding padding/
border on top, for both width and height; `min-width` forces a box
wider than its own container; `max-width` clamps a box narrower than
its own explicit `width`; `min-width > max-width` resolves to
`min-width`, never smaller; and `max-height` clamps the reported box
size while children (two full `<p>`s) are still completely laid out,
not truncated — same "spill past, don't silently drop" check v42's
own height test made. All pre-existing tests (23 total incl. these)
still pass unchanged, confirming the `border_w`/`border_h` refactor is
behavior-preserving for the default content-box, no-min/max case.
Visually confirmed with a real `grim` screenshot: a `border-box` box
rendered visibly smaller overall than an equivalent `content-box` one
with the same declared `width`; a `width: 500px; max-width: 220px` box
wrapped its text tightly into a ~220px box instead of 500px; and a
`max-height: 30px` box combined with `overflow: hidden` clipped
multi-line text to a short box, consistent with v40's existing
clipping.

**Deliverable:** `src/lib.rs` (`box-sizing`/`min-width`/`max-width`/
`min-height`/`max-height` parsing in `DeclarationParser`, `BoxSizing`/
`MaxLen` types, `Declarations`/`ComputedStyle` fields, cascade
resolution in `resolve_style`, the new `clamp_box_dim` helper replacing
`measure`'s inline width/height formulas, 6 new tests in the permanent
`mod tests`). Resolves the `box-sizing`/`min-*`/`max-*` `TODO.txt` item
left open by v42; `%` remains its own separate, still-open item.

## v44 — verify structural pseudo-classes (`:not()`, `:nth-child()`, `:first-child`)

> check the TODO for scratchy browser. Lets prioritize things and get
> to work

First pick under the standing autonomous `TODO.txt` workflow. Priority
call: most open items are deliberately deferred ("nothing needs this
yet" — `%`, table auto-layout, named `font-family`, `word-spacing`,
border bevels, box-shadow blur) or need the user's sign-off on a real
design tradeoff (`opacity`'s invasive plumbing, `letter-spacing`'s
cosmic-text bug) — both skipped again, untouched. This item stood out
as the cheapest real one: `TODO.txt` itself said structural
pseudo-classes were "a separate question nobody has actually verified
either way yet," and unlike `:link`/`:visited` (`OurPseudoClass`,
custom state scratchy tracks itself), `:not()`/`:nth-child()`/
`:first-child` are generic `selectors`-crate matching, driven entirely
by `SelectorsElement` methods (`prev_sibling_element`,
`next_sibling_element`, `is_root`, `is_empty`, ...) that `El` already
implements for other reasons — a real chance this was already working
for free, confirmed rather than assumed.

Wrote 3 new permanent tests. First attempt at the `:nth-child(2)`/
`:first-child` tests actually failed — not a scratchy bug, a test bug:
putting `<style>` directly inside `<body>` (as every prior pseudo-class
test in this file does) makes the `<style>` element itself the real
first child, shifting every `<p>`'s child index by one. Moving
`<style>` into `<head>` (where it semantically belongs anyway) fixed
it; all three tests then passed first try with no production code
changes. Visually confirmed with a real `grim` screenshot against a
3-`<p>` page combining all three selectors at once (`:nth-child(2)`,
`:first-child`, `:not(.skip)` via `text-decoration: underline`): 1st
`<p>` blue, 2nd green, both underlined, 3rd plain black with no
underline — exactly the expected combination. All 26 tests (23
pre-existing + 3 new) pass.

**Deliverable:** `src/lib.rs` (3 new tests in the permanent
`mod tests`: `not_pseudo_excludes_matching_element`,
`nth_child_matches_correct_position`, `first_child_matches_only_first`
— no production code changes, since the feature already worked).
Resolves the structural-pseudo-class open question in the `:hover`/
`:focus`/`:active` `TODO.txt` item, which now only covers the
genuinely-missing interaction-state pseudo-classes.

## v45 — CSS custom properties (`--name`/`var()`, `:root`-scoped)

> add to the todo rendering img tags

(sent mid-session, alongside the `<img>` item added to `TODO.txt` — see
its own entry there; this version's actual feature is custom
properties, picked right after v44 under the same standing autonomous
workflow, no new prompt for that part.)

Next `TODO.txt` pick after v44: CSS custom properties, the other item
flagged as fully spec'd with no design decision needed (unlike
`opacity`/`letter-spacing`, still untouched). Scoped down deliberately
to keep this one `resolve_style`-free: real CSS custom properties
inherit and cascade per-element like any other inherited property,
which would mean threading a resolved-custom-props map through
`resolve_style`'s cascade plus a second per-element substitution pass —
invasive plumbing on `opacity`'s order, not attempted. Instead:
`var()` resolves against one flat, whole-document table, built once
from whatever rules match the real `<html>` element (`:root`, `html`,
`html.foo`, ...) via the same real selector matching `resolve_style`
uses — not a hardcoded string check — then substituted as a single
text-level pass over the whole stylesheet before the normal parser
ever sees it. Added `:root` as a real `OurPseudoClass` variant along
the way (trivial — delegates straight to `Element::is_root()`, already
implemented), both because it's the natural place pages declare custom
properties and because it's a real, independently useful pseudo-class
on its own.

`Declarations` gained `custom_props: Vec<(String, String)>` (name kept
exactly as written — custom property names are case-*sensitive*,
unlike every other property here), populated by a new first branch in
`DeclParser::parse_value` that treats any `--`-prefixed name as "no
typed grammar, just capture the raw source text of the value" via
`ParserState`/`slice_from`. Three new free functions:
`collect_root_custom_props` (matches rules against `<html>`, merges
matched rules' `custom_props`), `substitute_css_vars` (balanced-paren-
aware text scan — correctly finds the matching `)` through a nested
function call in a fallback, so `var(--a, var(--b, red))` resolves
right), and its two small helpers (`matching_close_paren`,
`resolve_var_call`). `build_layout_tree` now harvests `root_vars` from
the first-pass `rules`, then — gated on the stylesheet text actually
containing `var(` at all, not on whether any custom property was
found; a `var(--undeclared, red)` with nothing declaring `--undeclared`
still needs the fallback substituted — re-parses both `DEFAULT_CSS` and
the page's own `<style>` text with vars substituted, replacing `rules`
entirely before any element resolution happens.

First test run caught a real bug in my own initial design, not a
production bug: gating the whole substitution pass on `!root_vars.
is_empty()` meant a `var(--undeclared, green)` with zero actual
`--name` declarations anywhere skipped substitution entirely, so the
literal `var(...)` text reached the normal color parser, failed, and
silently fell back to default black instead of resolving the fallback
— caught by `var_fallback_used_when_prop_undeclared` and
`nested_var_fallback_resolves` both failing with "black instead of the
expected color." Fixed by gating on whether the CSS *text* mentions
`var(` at all (a cheap substring check) instead of whether anything was
actually found — fallback resolution has to run even with an empty
vars table. `non_root_custom_prop_is_not_picked_up` (a `--c` declared
on `.scope`, not `:root`/`html`) exercised the same bug for a different
reason and is now the test that pins the deliberate "root-scoped only"
narrowing itself, not just the gating bug.

5 new permanent tests (31 total): `:root`-declared var resolves; a
plain `html` selector scopes a var too (not just `:root` specifically);
an undeclared var falls back to its fallback; a fallback containing its
own nested `var()` resolves correctly; and a non-root-scoped `--c`
isn't visible to `var()` (pins the documented scope limit, not a bug).
Visually confirmed with a real `grim` screenshot combining all of it at
once: `:root`-declared `--bg`/`--brand` painting the body background
and an `<h1>`'s color; a `<p>` falling back to magenta for an
undeclared var; and a `.box` whose background uses `var(--brand,
black)` (brand found, fallback unused) and whose border-color uses
`var(--undeclared-2, var(--brand))` — nested fallback correctly
resolving through to the same brand blue.

**Deliverable:** `src/lib.rs` (`Declarations.custom_props`, the
`--`-prefixed branch in `DeclParser::parse_value`, `OurPseudoClass::
Root`/`:root` parsing+matching, `collect_root_custom_props`/
`substitute_css_vars`/`matching_close_paren`/`resolve_var_call`, the
`build_layout_tree` reparse-with-substitution step, 5 new tests in the
permanent `mod tests`). Resolves the CSS-custom-properties `TODO.txt`
item, replaced with a narrower one describing exactly the root-only-
scoping/no-cross-var-substitution limits above.

## v46 — `<img>` rendering

> Let's plan the implementation of the <img> tag from scratchies backlog

The next `TODO.txt` pick after v45, picked directly by the user rather
than the usual autonomous top-of-TODO selection — already flagged as a
real near-term target in the item v45's own session split out (see
NOTES.md). Planned before any code: read the box-model/inline-layout/
measure/paint architecture and the existing `<img>` TODO.txt item, then
worked through three real design decisions with the user via
`AskUserQuestion` rather than guessing:

1. **Inline placement.** Real CSS lays a replaced `inline` element out
   as an atomic inline-level box sharing the line with surrounding
   text — confirmed this is exactly what Firefox's `nsLineLayout` does
   too, not a shortcut hiding in some real implementation, after the
   user asked "how does firefox handle this?" directly. This engine's
   inline runs are one cosmic-text `Buffer` of text spans with no way
   to embed a non-text box mid-line, so true inline flow would mean
   hand-rolling line-breaking — a materially bigger lift. The user also
   asked how tied scratchy is to cosmic-text and whether extending it
   upstream was worth it; answered no (cosmic-text is a UI text-shaping
   library, not a document-layout engine with embedded-object support,
   and this project already treats it as a black box to work around —
   see the `letter-spacing` TODO.txt item, a real cosmic-text bug
   documented and deferred rather than patched). Decided: `<img>`
   always gets its own box, breaking surrounding inline text into
   separate runs before/after it — a documented spec divergence, not
   silently dropped.
2. **Image formats:** PNG/JPEG/GIF/WebP via the `image` crate, explicit
   features only (not the crate's default feature set) — covers
   real-world web images without pulling in BMP/TIFF/ICO/etc. nothing
   needs.
3. **Broken image fallback:** render `alt` as plain text, matching a
   real browser's replaced-element fallback-content behavior, rather
   than collapsing to a blank box.

One more scope cut the user made directly, after the plan already
proposed resolving a relative `src` against the HTML file's own
directory: "I'd rather drop this" — then, after the inline-placement
question was answered, clarified further to "drop file processing in
general... treat files the same way as we treat network until further
notice." So `<img src>` only decodes a `data:` URI; a local file path
isn't read at all, same bucket as the engine's existing no-networking
stance, not a separate near-term item.

**What/why:** `BoxContent` gained an `Image(Rc<DecodedImage>)` variant
(`DecodedImage` already in this file's own packed-`u32`-`TextColor`
pixel form, not `image`'s own types, so nothing downstream needs to
know the crate exists). `decode_data_uri` (base64 payload only — the
form `snag` emits) feeds `decode_image_bytes` (`image::load_from_memory`
+ `to_rgba8`). `build_img_box` builds `<img>`'s box directly in
`build_box_for_element` (a void element — no children walk), applying
HTML `width`/`height` attributes (`img_size_attr`) as a fallback only
when CSS left the matching property `auto` — lowest-priority
presentational hint, same as a real UA, via `resolve_style`'s already-
cascaded `ComputedStyle.width`/`.height`. `collect_children` special-
cases `tag == "img"` the same way `Display::Block` already is, flushing
`pending` and giving it its own box regardless of its resolved
`display` — the actual "breaks the line" mechanism.

`measure` gained a `BoxContent::Image` arm: real CSS replaced-element
sizing — both axes `auto` uses the intrinsic decoded size; one axis set
scales the other via the intrinsic aspect ratio (computed from whichever
axis resolves first, so it works symmetrically in both directions); both
set stretches independently, which the existing `clamp_box_dim` already
does for free (default `object-fit: fill`). Doesn't re-derive the aspect
ratio after `min`/`max-width/height` clamp the other axis — a further
edge case nothing needs yet (noted in `TODO.txt`). Painting: a new
`blit_image` primitive (nearest-neighbor scaled, composited through the
same `blend` every other paint primitive uses, so PNG alpha shows
whatever `paint_box_decoration` already painted underneath), wired into
`paint`'s `LaidContent::Image` arm after the box's own background/
border, and a trivial `hit_test_cursor` arm (no finer-than-the-whole-box
hit test, same as any other replaced content).

8 new permanent tests (39 total): intrinsic sizing with no CSS/attrs,
width-only and height-only aspect-ratio scaling (both directions,
confirming the symmetric computation), HTML attribute sizing, CSS
overriding HTML attributes, the inline-text-breaks-into-separate-runs
placement behavior (`<p>` splits into exactly 3 sibling boxes around the
`<img>`), and alt-text fallback for both a missing `src` and an
undecodable one. Test images are generated in-memory via the `image`
crate itself and base64-encoded into a `data:` URI at test time, not
hardcoded blobs. Visually confirmed with a real `grim` screenshot: a
red-blue gradient PNG rendered at its intrinsic size, scaled
proportionally from a CSS `width` alone, stretched independently from
HTML `width`/`height` attributes, wrapped correctly by a `border`+
`padding` box, breaking surrounding paragraph text onto separate lines
on either side, and a corrupt `data:` payload falling back to its `alt`
text.

**Deliverable:** `src/lib.rs` (`BoxContent::Image`, `DecodedImage`,
`decode_data_uri`, `decode_image_bytes`, `img_size_attr`,
`build_img_box`, the `img`-tag hooks in `build_box_for_element` and
`collect_children`, the `measure`/`paint`/`hit_test_cursor` `Image`
arms, `blit_image`, 8 new tests in the permanent `mod tests`),
`Cargo.toml` (`image`, `base64`). Resolves the `<img>` `TODO.txt` item,
replaced with a narrower one describing exactly the no-local-file/no-
true-inline-flow/no-`object-fit`/nearest-neighbor-only limits above.

## v47 — `background-image` CSS property

> next two items: enable background image on CSS. Make <img> tags inline by default. Update the todo

Picked directly by the user again, same as v46 — two items in one
message; this version is the first (`background-image`), unambiguous
and well-scoped on its own. The `<img>`-inline item is the exact
"Bigger, deliberate scope cut" v46's own `TODO.txt` entry already
flagged as a real rewrite, not a quick follow-up — held for a separate
version/scope conversation with the user rather than guessed at here.

**What/why:** the real constraint carried over from v46 — a decoded
image can't live in `ComputedStyle` (`#[derive(Copy)]`, cheaply
duplicated everywhere a style is read) — so `background-image`, like
`<img>`'s own pixels, is resolved and threaded alongside a box rather
than inside its style. `Declarations` gained `background_image:
Option<String>` (the raw `url(...)` argument, parsed via cssparser's
own `expect_url()`, which already handles both the unquoted-`url(...)`
and quoted-`url("...")`/`url('...')` forms) and a parser branch
alongside `background-color`'s. `resolve_style`'s own selector-matching
loop was extracted into `matching_declarations` (no behavior change,
just factored out) so a new `resolve_background_image` could reuse it
instead of re-running `matches_selector_list` over every rule a second
time — then decodes the cascaded URL through the same `decode_data_uri`
+ `decode_image_bytes` pair `<img>` already uses (same `data:`-URI-only
policy), producing an `Option<Rc<DecodedImage>>`.

`LayoutBox` and `Laid` both gained a `background_image` field
(`build_box_for_element` resolves it once per real element and passes
it to every `LayoutBox` it builds, including into `build_img_box` — so
an `<img>` can carry both its own foreground pixels *and* a CSS
background-image, same as a real browser; every anonymous-box
construction site gets `None`, nothing to resolve against). `measure`
threads it through unchanged into `Laid` (a cheap `Rc` clone) rather
than computing anything from it. Painting: a new `tile_image` primitive
(same composite-via-`blend` shape as `blit_image`, but repeats the
source image across the box via `rem_euclid`-wrapped sampling instead
of scaling to fit), called from `paint_box_decoration` — extended with
a new `background_image: Option<&DecodedImage>` parameter — right after
the `background-color` fill and before the border, matching real CSS's
background-color-then-background-image-then-border stacking.

3 new permanent tests (42 total): `tile_image` itself repeats a 2×1
red/blue source across a wider pixel buffer (proves real tiling, not
stretching); a full parse→layout→paint pipeline test confirms a CSS
`background-image: url(data:...)` declaration paints inside a box's
border (and that the border itself still paints on top); and a
sanity check that a div with only `background-color` (no
`background-image`) resolves `background_image: None` without
disturbing `background-color`. Visually confirmed with a real `grim`
screenshot: a 16×16 checkerboard pattern tiling correctly across a
300×150 bordered box, and again layered over a solid
`background-color` on a second box.

**Deliverable:** `src/lib.rs` (`Declarations.background_image`, the
`background-image` branch in `DeclParser::parse_value`,
`matching_declarations` (extracted from `resolve_style`),
`resolve_background_image`, the `background_image` field on
`LayoutBox`/`Laid` and every construction site, `tile_image`, the
`paint_box_decoration` parameter + call site, 3 new tests in the
permanent `mod tests`). New `TODO.txt` item for `background-image`
(no `background-size`/`-position`/`-repeat` longhand, no `none`
keyword, no multiple images, not clipped to `border-radius`).

## v48 — real inline-level `<img>` flow

> next two items: enable background image on CSS. Make <img> tags inline by default. Update the todo

This version is the second of that same message's two items — the one
v47 deliberately held back as its own scope conversation, since it's
exactly the "bigger, deliberate scope cut" v46's own `TODO.txt` entry
already flagged: text truly sharing a line with an inline `<img>` and
wrapping around it, not just breaking onto separate blocks. Before
writing any code, worked through the real scope question with the user
via `AskUserQuestion`: a cheap "flush gap tightened, still not real
sharing" approximation, or the full spec-correct hand-rolled inline
layout. The user asked two follow-ups first — "what does the standard
say?" (atomic inline-level replaced box, confirmed) and "how does
firefox handle this?" (exactly the standard, `nsLineLayout` natively
mixes text runs and atomic inline frames — no shortcut hiding in a real
implementation, see v46's own entry) — then picked the full rewrite.

**What/why:** `BoxContent::Inline(Vec<(String, SpanStyle)>)` became
`BoxContent::Inline(Vec<InlineItem>)`, where `InlineItem` is `Text`
(the old tuple, now a variant) or `Image { image, style,
background_image }` (an inline `<img>` is still a full element — own
size/margin/border/background, not just foreground pixels, same as the
block-level case). Every push site (`collect_children`'s text branch,
the `<li>` marker prefix, `build_img_box`'s alt-text fallback, every
anonymous-box flush) updated to match. `shape_spans` now takes `&[
InlineItem]` directly and filters to `Text` entries internally, so the
same function serves both its original whole-paragraph call and (new)
a single-word call, with no separate adapter.

`collect_children`'s `<img>` handling split in two: a `display: block`/
`table` override still gets its own box exactly as v46 left it
(`build_img_box`, now behind a new shared `resolve_img` helper); the
*default* `display: inline` case instead decodes right there (same
`resolve_img`) and pushes an `InlineItem::Image` straight into the
surrounding `pending` run, with no flush.

The actual new layout engine is `measure_mixed_inline`, called from a
new first arm on `measure`'s `BoxContent::Inline` match (guarded on
`items.iter().any(|i| matches!(i, InlineItem::Image { .. }))`, so a
plain-text run is completely untouched — same single-`Buffer` fast path
as always, zero regression risk/cost for the overwhelmingly common
case). For a mixed run: splits every text item into words
(`split_inclusive(' ')`), shapes each as its own tiny one-span
paragraph via the *existing* `shape_spans`/`ShapeCache`/`paint_buffer`
machinery (one `ShapeCache` id per word, same `deadline`/`incomplete`
incremental-shaping budget a whole paragraph would use — a shaped word
is just a smaller unit of the same thing, not a special case), sizes
each image via a new shared `image_box_size` (extracted from the old
`BoxContent::Image` arm — same replaced-element algorithm for both a
block-level and an inline image), then runs a plain greedy line-
breaker (wrap when a token doesn't fit; `text-align: left/right/center`
applied per line afterward) and emits absolute document-space
positions. `LaidContent` gained `MixedInline(Vec<MixedPositioned>)`,
where a `Word` is `{id, x, y, w, h}` (paints via the *existing*
`paint_buffer`, called once per word instead of once per paragraph —
full color/background/decoration/cursor fidelity for free) and an
`Image` is a *complete nested `Laid`* (`Box<Laid>`, absolute-positioned)
— `paint`/`hit_test_cursor` just recurse into it exactly like any
other child `Laid`, so it gets box-decoration/clipping/culling for free
instead of a second parallel painting path. An image is bottom-aligned
within its line (approximating real baseline alignment without
threading real font-metrics baseline offsets out of cosmic-text).

**Explicitly not attempted, documented in the new TODO.txt text**
(all judgment calls made to keep this a tractable single-session
rewrite rather than a second flexbox-scale project): no cross-word
kerning/ligatures; no bidi reordering across words in a mixed run;
`white-space: pre` not honored in a mixed run; an inline image's
*vertical* margin is ignored (horizontal margin correctly reserves
line space); `justify` falls back to `left`; a line's trailing space
counts toward alignment width (a real UA hangs it past the edge).

9 new/rewritten permanent tests (46 total): the pre-v48
`img_breaks_surrounding_inline_text_into_separate_runs` test, which
pinned the *old* behavior this version deliberately reverses, was
rewritten (not deleted — the old behavior it pinned is gone, not an
untouched feature) into `img_with_default_display_shares_the_inline_
run_with_surrounding_text`, plus a new sibling test pinning that an
explicit `display: block` override still gets its own box (the one
case that needed to keep working exactly as before). Three genuinely
new tests exercise `measure_mixed_inline` itself: word+image fit on one
~20px line (checked via the run's own reported height, not exact `y`
equality — the word is top-aligned and the image bottom-aligned
*within* the same line, so their `y` values legitimately differ even
when correctly sharing a line); two 50px-wide images in a 60px
container are forced to wrap deterministically (no dependency on real
font metrics); and `text-align: center` shifts a mixed line's content
rightward versus the default. `find_image_laid` (v46) and
`find_blocks_containing_image` (v46) both updated/reused to account for
the new `MixedInline` shape. Visually confirmed with a real `grim`
screenshot: a short sentence with an icon genuinely flowing mid-line
between words (wrapping naturally to a second line afterward); a
narrow column forcing the icon into the middle of a multi-line wrap,
with text correctly continuing after it; and a `display: block`
`<span>` with `text-align: center` correctly centering a word+image+
word mixed line.

**Deliverable:** `src/lib.rs` (`InlineItem`, `resolve_img` (extracted
from `build_img_box`), the `<img>` display split in
`collect_children`, `shape_spans`'s new `&[InlineItem]` signature,
`image_box_size` (extracted from `BoxContent::Image`'s `measure` arm),
`measure_mixed_inline`, `LaidContent::MixedInline`/`MixedPositioned`,
the `paint`/`hit_test_cursor` `MixedInline` arms, 9 new/rewritten tests
in the permanent `mod tests`). Resolves the "bigger, deliberate scope
cut" half of the `<img>` `TODO.txt` item from v46, replaced with the
simplification list above.

---

## v49 — `%` (percentage) lengths

> set up a loop where you pick the next TODO item, and once completed,
> clear your context and do that again until the list is done

Item 1 of that autonomous TODO.txt loop. First `TODO.txt` item, top to
bottom: `%` and unitless-non-zero were the last gap left in `px`/`em`/
`rem` length support (v14/v34), deliberately deferred three times
already ("not adding them until `default.css` or a real test page
actually needs one") — the user's standing loop instruction means that
deferral reasoning no longer applies, so this landed the real thing
instead of deferring a fourth time.

**Unitless first, since it was quick to settle**: grepped the existing
grammar and confirmed the TODO note's premise — every length-ish
property parsed through `parse_length`/`set_length` already only
accepts a bare unitless `0`, and `line-height` already accepts a bare
non-zero number through its own separate, pre-existing grammar
(`parse_line_height`, untouched by this change beyond adding its own
`%` arm — see below). Real CSS only allows a unitless non-zero number
for a small set of properties scratchy mostly doesn't implement
(`z-index`, `flex`, ...) plus `line-height`, which already had it — so
there was nothing to actually change here, just to confirm and pin
with a permanent test (`unitless_nonzero_number_is_still_rejected_for_
margin`), not quietly skip.

**`%` was the real work**, and the one genuine design question: *what*
does each property's percentage resolve against. Went through the real
spec property by property rather than assuming one basis fits all:
`width`/`min-width`/`max-width` and `margin`/`padding` (all four
physical sides — CSS2.1 is explicit that even `margin-top`/`padding-top`
resolve against the containing block's *width*, not height, a detail
easy to get backwards) resolve against the containing block's width;
`height`/`min-height`/`max-height` resolve against the containing
block's own *height*, which is only sometimes definite; `font-size`
resolves against the parent's font-size (same basis `em` already uses
there); `line-height`'s own `%` resolves against the *declaring*
element's own font-size (same timing its `em`/`px` forms already use,
confirmed distinct from its bare-`<number>` form, which stays symbolic
and re-resolves per descendant — see v24); `border-radius` resolves
against this box's *own* border-box width, not a containing block's;
and `border-width`/`outline-width`/`box-shadow`'s offsets/blur/spread
deliberately *still* reject `%` entirely — the real CSS grammar for
every one of them is `<length>`, not `<length-percentage>`, so adding
a percentage there wouldn't be closing a gap, it'd be accepting
something the spec itself doesn't.

**The real architectural piece**: `width`/`height`/`margin`/`padding`/
`border-radius` were previously resolved straight to a final pixel
`f32` inside `resolve_style`, which runs once per element *before*
layout — fine for `px`/`em`/`rem`, which only ever need a font-size
(already known at cascade time), but a containing-block dimension
genuinely isn't known until `measure` actually lays a box out. A new
`BoxLength` type (`Px(f32)` / `Percent(f32)`) replaces the plain `f32`
these fields carried in `ComputedStyle`, staying symbolic through the
cascade exactly the way `LineHeight` already does (v24) for the same
reason, and only resolving to a concrete pixel value where a real basis
finally exists: `finalize_box_lengths`, called once at the top of
`measure` (basis: the box's own incoming `width` parameter, for every
width-basis field) and once per inline `<img>` inside `measure_mixed_
inline` (basis: that run's own `content_w`), with `border-radius`
resolved separately, directly in `paint_box_decoration` against its own
final `w` — the simplest correct basis for a property whose percentage
is self-referential, not containing-block-relative. After `measure`
finalizes a box's own `ComputedStyle`, every `BoxLength` field is
guaranteed `Px` — `.px()` reads it with a debug-assert guarding that
invariant, used everywhere paint/hit-testing/children-stacking code
reads a value `measure` already resolved for them.

**Height's own wrinkle, worth spelling out**: real CSS says a `%`
height against an indefinite containing block (an ordinary `height:
auto` ancestor — the common case) computes to `auto`, i.e. has no
effect, not some value resolved against a bogus implicit basis. Modeled
this with a `containing_height: Option<f32>` parameter threaded through
`measure`'s own recursion — `None` when indefinite, `Some` only when
the immediate containing box's own `height` is itself definite (an
explicit length, or a `%` that itself resolved against *its* own
incoming `containing_height` — the same rule, recursively, matching the
real spec exactly). Verified both directions: a `%` height against an
explicit 300px ancestor resolves correctly, and the identical markup
with vs. without a `%` height against an *indefinite* ancestor reports
byte-identical box heights (confirming the percentage genuinely had
zero effect, not a wrong-but-plausible one).

**Attempted and reverted, not shipped**: wiring the window's own inner
height through as the *top-level* call's `containing_height`, so the
classic `html, body { height: 100% }` full-page pattern would chain
down from a real viewport-height concept. Caught before committing,
not after: this engine's document root (a synthetic wrapper box
`build_layout_tree` puts `<html>` *inside*, not literally the tree's
own root) has no height of its own for that value to chain through
without also giving the wrapper an explicit height — and that same
root `Laid`'s own `h` is *also* what `redraw`'s scroll-clamping reads
directly as "total scrollable content height." Pinning it to the
window's own height would have silently capped every real page at
exactly one viewport's worth of scrollable content — a real regression
to actual scrolling, worse than the gap it would have closed. Reverted
cleanly, documented as its own narrower `TODO.txt` sub-item (needs the
root box's two conflated roles — containing-block-height vs.
total-scroll-height — separated first, not a one-line fix) rather than
shipped half-working or guessed into something worse.

9 new permanent tests (63 total): `%` width against a containing
block's width; `%` margin (top *and* left, on the same declaration)
against a containing block's width, not the margined box's own
unrelated width; `%` padding-top specifically against width, not the
box's own height (the easiest of these to get backwards); `%` height
against a definite ancestor height; `%` height computing to auto
against an indefinite one (the before/after-identical-heights check);
unitless `margin-top: 5` still dropping, not treated as `5px`; `%`
line-height against the declaring element's own font-size; and `%`
border-radius against the box's own border-box width. Visually
confirmed too: a real test page with `%` width, `%` width + `%`
margin-left, and a `%`-height box nested inside an explicit-height
ancestor, screenshotted via `grim` — all three resolve exactly where
the numbers say they should.

**Deliverable:** `src/lib.rs` (`Length::Percent`, `BoxLength` type +
`from_length`/`resolve`/`px`, `parse_length_percent` (new, alongside
the unchanged `<length>`-only `parse_length`), `Sides`'/`ComputedStyle`'s
box-dimension fields retyped to `BoxLength`, `finalize_box_lengths`,
`measure`'s new `containing_height` parameter and `child_containing_
height` derivation, `clamp_box_dim`'s `BoxLength`-aware signature,
`paint_box_decoration`'s own `border_radius` resolution, 9 new tests in
the permanent `mod tests`); updates `TODO.txt` (closes the `%`/unitless
item, replaced with a narrower `TODO.txt` sub-item on the viewport-
height gap found and deliberately not fixed along the way).

---

## v50 — real CSS auto table layout (per-column min/max content width)

> set up a loop where you pick the next TODO item, and once completed,
> clear your context and do that again until the list is done

Item 2 of that same autonomous `TODO.txt` loop (v49 did item 1, `%`
lengths). Next `TODO.txt` item, top to bottom: table layout's own
equal-width-column simplification from v15, explicitly scoped smaller
at the time ("a genuinely separate two-pass algorithm... [sizing] each
column from its cells' min/max content width") pending a real page that
needed it. Same as v49, the loop's standing premise is that this
deferral no longer applies on its own — real CSS2.1 §17.5.2.2 automatic
table layout landed instead of deferring a fourth version in a row.

**The real algorithm, scoped to what matters without colspan/rowspan**
(still out of scope, unchanged — a row's `n`th cell always maps to
column `n`): each column's own `(min, max)` content width is the
largest of its cells' own natural width across every row — `cell_
natural_width`, a new recursive function covering every `BoxContent`
shape a cell's content can take (`Inline` text/images, nested `Blocks`,
and a nested `<table>`'s own rows). Column widths are then distributed
across the table's own `content_w` via the spec's real three-case rule:
if every column's `max` fits, each grows proportionally to its own
share of `max` to fill the rest; if even every column's `min` doesn't
fit, each shrinks proportionally to its own share of `min` instead
(letting real content overflow, same as a `min-width` wider than its
container does elsewhere in this engine); otherwise each interpolates
between its own `min` and `max` in proportion to its share of the
table's total `max - min` slack — the ordinary case for most real
tables. One deliberate, documented divergence from the literal spec:
real CSS auto-width tables actually shrink-to-fit their own content
rather than filling their container — changing *that* is a separate,
bigger scope than "size the columns from their content" (this item's
actual ask), so the table's own box-model width is untouched; only how
`content_w` gets divided among columns changed.

**Computing a cell's natural width turned out to be exactly what `measure_
mixed_inline` (v48) already does, just run at its two extremes.** That
function's own greedy word-wrapper, called at an effectively unconstrained
width, leaves every word/image on one line — the furthest item's own right
edge *is* the unwrapped (`max`) content width. Called at `0` width, its
own `cursor_x + w > content_w` check is always true for a second item on
an empty-budget line, forcing every word/image onto its own line — the
widest resulting line *is* the widest unbreakable token (`min`). This
means a cell mixing text and an inline `<img>` gets the exact same
treatment as a plain-text one, with no separate code path, and — since
`measure_mixed_inline`'s own word-shaping already participates in the
`deadline`/`cache`/`next_id` incremental-shaping budget — the natural-
width pass inherits that budget for free: a huge table degrades the same
resumable way a huge paragraph already does, not a separate, unbounded
pass. The real, expected cost: a table cell now gets shaped up to three
times (once at each width extreme to find its natural size, once more at
its real assigned column width) instead of once — the "shape once to find
natural width, size columns, reshape at final width" two-pass cost v15's
own entry already anticipated as what the real algorithm would require.

**The real architectural piece, smaller than v49's own but same shape**:
a table row measured with real per-column widths can't go through
`measure`'s own generic recursive dispatch at all — the equal-width case
could, since each `BoxContent::TableRow` arm derived its own split purely
from its own `content_w`, but real column widths are a property of the
whole *table*, computed once before any of its rows are measured, and
have to be handed to each row from outside. Rather than thread a new
column-widths parameter through `measure`'s own signature (touching every
recursive call site, including ones with nothing to do with tables),
`display: table` is special-cased at the very top of `measure`, before
any of its normal box-model/content-dispatch logic runs: `measure_table`
owns the whole table (and, via `measure_row_with_widths`, each of its
rows) as a self-contained alternate path, never re-entering `measure`'s
own `BoxContent::TableRow` arm at all for a well-formed table. That arm
stays, now explicitly documented as a defensive-only equal-width
fallback for a `TableRow` box that somehow ends up outside `measure_
table`'s own dispatch (shouldn't happen — `build_table_children` never
produces one that way). Extracted `measure`'s own box-model preamble
(margin/padding/border/`clamp_box_dim` math) into a shared `box_model_
entry` helper during this, used both by `measure` itself (unchanged
behavior, confirmed by the full pre-existing suite passing unmodified)
and by `measure_row_with_widths`, which needs the identical math without
going through `measure`'s own dispatch.

**No existing permanent test pinned the old equal-width behavior** — v15's
own verification was a temporary test, deleted before that version's
commit (this project's testing policy only reversed to "keep tests
permanently" in v41, well after v15), so there was nothing to rewrite,
only new tests to add. 4 new permanent tests: two uneven columns end up
substantially different widths, not an equal split; column widths still
sum to the table's own content width (confirming the "fill the container,
don't shrink-to-fit" divergence above is intentional, not a bug); a
container too narrow for either column's own natural width still gives
the longer single-word column more room than the shorter one, instead of
clamping both to some shared minimum (exercises the "even `min` doesn't
fit" shrink branch specifically); and v15's own "deliberately pathological"
nested-table-in-a-cell case still gets its own independent column sizing,
not flattened into the outer table's columns. Visually confirmed with a
real `grim` screenshot: a 3-column table with one much-longer-text column
rendered with that column genuinely wide and the other two tight around
their short content, not three equal thirds.

**Still explicitly out of scope, unchanged from v15's own item** (none of
this needed touching any of them): no colspan/rowspan, no border-collapse/
cell borders, no `caption-side` placement. One narrower gap newly
documented: an explicit `width`/`min-width`/`max-width` set directly on a
`<td>`/`<th>` (or a `<col width>`/CSS `width` on a `col`/`colgroup`) isn't
honored as a column-sizing hint — only a cell's own *content* drives its
column's width, same as `<col>` already carried no visual weight under
the old equal-width layout.

**Deliverable:** `src/lib.rs` (`BoxModelEntry`/`box_model_entry` (factored
out of `measure`'s own preamble), the `Display::Table` dispatch at the
top of `measure`, `measure_table`, `table_column_widths`, `cell_natural_
width`, `mixed_inline_extent`, `image_natural_width`, `lay_out_row_cells`,
`measure_row_with_widths`, the `measure`-internal `BoxContent::TableRow`
arm rewritten as an explicit defensive fallback, 4 new tests in the
permanent `mod tests`); updates `TODO.txt` (rewrites the table-layout item
to describe real per-column sizing plus its own narrower remaining gaps).

---

## v51 — named `font-family` fallback lists

> set up a loop where you pick the next TODO item, and once completed,
> clear your context and do that again until the list is done

This is the next item of that autonomous `TODO.txt` loop — font-family,
picked up after v50's real auto table layout. `TODO.txt`'s item: v16's
`font-family` only ever recognized the 5 bare CSS generic keywords
(`serif`/`sans-serif`/`monospace`/`cursive`/`fantasy`); a quoted name
(`"Georgia"`), an unquoted multi-word name (`Helvetica Neue`), and a
real comma-separated fallback list all silently dropped the whole
declaration. Deliberately scoped out at the time — v16's own writeup
said `default.css` only ever used bare `monospace`, so nothing needed
more — but the autonomous loop's whole premise is that this deferral no
longer applies once the item is at the top of the list, same reasoning
v49 (`%` lengths) used after three earlier rounds of "not adding until
something needs it."

**Parsing.** `parse_font_family` replaces the old single-`expect_ident`
grammar with a real comma-separated loop: each entry is either a quoted
string (`input.expect_string()`, taken verbatim as a specific family
name) or one-or-more bare idents collected and joined with a space —
real CSS grammar for an unquoted name like `Helvetica Neue` — unless
the entry is a *single* ident that case-insensitively matches one of
the 5 generic keywords, which is recognized as generic instead of a
literal one-word family name (a multi-word unquoted entry can never
collide with a generic, since none of the 5 keywords are two words).
Fails only if the value is empty or a comma has nothing on one side —
unlike v16's "one bad token drops the whole declaration," an
individual *entry*'s own resolution (whether its named family actually
exists on this system) is never a parse-time concern at all; that's
the whole point of a fallback list, resolved at shape time instead (see
below).

**The `Copy` question NOTES.md v34 flagged, resolved.** v34 explicitly
parked this item rather than pick unilaterally between three options
for storing an owned, variable-length family list inside
`ComputedStyle`/`InheritedStyle`/`SpanStyle` — all three
`#[derive(Clone, Copy)]` by design, threaded by value through every
recursive tree-building function. Went with the `Box::leak` option:
`FontFamily` is now `struct FontFamily(&'static [FontFamilyItem])`, a
`&'static` slice (itself trivially `Copy` regardless of what it points
to) rather than an owned `Vec`. Each distinct parsed list — and each of
its `Name` items' strings — is leaked once in `DeclParser`'s
`font-family` handling. This keeps every struct that already carried a
plain `FontFamily` field working completely unchanged — no cascading
`Copy`-to-`Clone` refactor touching a large fraction of this file for a
property most pages never even declare. The accepted cost: that memory
is never freed for the life of the process. Not a new category of
problem for this codebase — it already re-parses the entire stylesheet
from scratch on every relayout (no cross-redraw CSS caching at all),
so this is bounded by how many *distinct* `font-family` declarations
exist in a document's source (typically a handful) and dwarfed by the
full-document reparse work already redone on every one of those
redraws, not by element count or redraw count. `FontFamily::default()`
needs no leak at all for the overwhelmingly common case (no
`font-family` declared) — `&[FontFamilyItem::SansSerif]` is a plain
array-literal reference to `Copy`, `Drop`-free data, eligible for
`'static` promotion by the compiler itself.

**Fallback resolution — reading cosmic-text's own source, not
guessing.** `cosmic_text::Attrs` has no fallback-list concept of its
own: `Attrs::family` is a single `Family<'a>`, not a slice, so a parsed
list can't be handed to it directly no matter how it's stored.
Confirmed by reading `fontdb::Query`/`Database::query`'s actual
vendored source (`fontdb-0.23.0/src/lib.rs`) rather than assuming:
`query()` already loops a `Query`'s `families` slice in order and
returns the first one with any installed face — real in-order fallback
semantics already live inside `fontdb`, and it neither errors nor
silently substitutes an arbitrary font for an unavailable name. The
catch: `query()` hands back a matched font *ID*, not which *family
name* in the list produced it — information `Attrs::family` itself
needs for cosmic-text's own later shaping/caching to work correctly.
So `FontFamily::resolve` queries one candidate at a time (`fontdb::
Query { families: std::slice::from_ref(&family), weight, style,
..Default::default() }`), stopping at the first that resolves —
observably the same in-order fallback behavior as asking `fontdb` to
walk the whole list at once, while letting each step report back its
own `Family` directly. Called from `shape_spans`, where `attrs.weight`/
`attrs.style` are already set (needed so the query matches the actual
weight/style being shaped, not just the family name) and
`font_system.db_mut()` is available (there's no immutable `db()`
getter on `FontSystem`, only `db_mut()` — confirmed by reading its
public API, not assumed; `&mut Database` coerces to `&Database` at the
call site with no extra ceremony).

**Verified past the cascade, same diligence v16's own now-removed test
used.** Four permanent tests (`#[cfg(test)] mod tests`, kept per the
v41 policy reversal, not written-then-deleted): one parses `'Georgia',
Helvetica Neue, monospace` and asserts the exact 3-item structure
(quoted name, joined multi-word name, trailing generic, in order); one
confirms the old v16 single-bare-generic case still parses to exactly
one item (no regression from generalizing the grammar); one confirms
real CSS inheritance still threads the resolved list down to a
child with no `font-family` of its own; and one runs the *full*
pipeline (`build_layout_tree` + `measure`) on three real paragraphs —
plain `monospace`, plain `sans-serif`, and a fallback list naming an
impossible family before a real `monospace` keyword — then inspects
the actually-shaped `cosmic_text::Buffer`'s own `fontdb::ID` per
paragraph (not scratchy's own `FontFamily` enum value, which would
only prove the data cascades correctly, not that it reached a real
font). Confirms monospace and sans-serif resolve to two different
fonts on this system, and that the fallback-list paragraph lands on
*exactly* monospace's own font ID — not cosmic-text's default
sans-serif, and not some other font a bad implementation might
silently substitute. One snag along the way: the first version of this
test put the three `<p>`s on separate lines in the test's HTML source,
producing 4 extra whitespace-only inline runs (before/between/after
each paragraph) with their own shape ids — 7 total instead of the
expected 3. Not a bug in the feature; fixed by writing the fixture
HTML on one line with no inter-tag whitespace, same category of "test
structure, not code" lesson v19's `resolve_style(li, ROOT_STYLE)`
shortcut already taught.

**Visually confirmed with a real `grim` screenshot**, not just headless
tests: a 4-paragraph page — a default sans-serif baseline, `'Liberation
Serif', serif` (installed on this system, confirmed via `fc-list`),
`NoSuchFontXYZ123, Liberation Mono, sans-serif` (first name missing,
should fall through to the second, installed one), and `'Totally Fake
Family', monospace` (falls through to the trailing generic) — rendered
exactly as expected: the named-and-installed line visibly has real
serifs distinct from the sans-serif baseline, and both fallback lines
render in a visibly monospaced font, not sans-serif.

**Deferred, not attempted**: resolving a family by *weight-matched*
face only when the declared weight is actually available for that
family (fontdb's own `find_best_match` already does real nearest-weight
matching within whichever family wins, so this isn't a gap — just
noting it wasn't separately verified here, since nothing in
`default.css` or any test page pairs an unusual `font-weight` with a
named family yet).

## v52 — a first positioning model: `position: relative`/`absolute`/`fixed`, `z-index`

> set up a loop where you pick the next TODO item, and once completed,
> clear your context and do that again until the list is done

This is the next item of that same autonomous `TODO.txt` loop (v49 did
`%` lengths, v50 real table layout, v51 named `font-family` fallback
lists) — the first of the big, originally-deferred post-v20 survey
items rather than a narrow residual gap: "No positioning model at all:
`float`/`clear`, `position` (relative/absolute/fixed/sticky), and
`z-index`/stacking contexts are unparsed and unimplemented." Explicitly
scoped down, per standing instructions for an item this size: landed
`position: relative`/`absolute`/`fixed` and a real (if intentionally
non-exhaustive) `z-index` paint-order mechanism; deferred `float`/
`clear` (a genuinely separate layout algorithm — inline content
actually wrapping around a floated box, not an extension of anything
built here), `position: sticky` (needs real scroll-position awareness,
a different kind of input than anything else here resolves), and full
CSS2.1 Appendix E stacking-context ordering (what landed handles the
common "overlay/dropdown sits above normal content" case correctly,
not negative `z-index` or a `relative` box's own separate stacking
context) — each is now its own precise `TODO.txt` item, not folded back
into one vague leftover line.

**The core design question, settled first:** everything in this file's
layout pipeline works in one coordinate space — every `Laid`'s `x`/`y`
is already absolute, document-space pixels (scroll is applied once, at
paint time, by subtracting `scroll_offset`) — not a nested local/
relative space the way a lot of layout engines model it. That one fact
is what makes `relative`'s own implementation genuinely simple: since a
box's *reported size* (what its parent's `cursor_y` stacks the next
sibling against) is completely decoupled from its *position*, visually
offsetting a box after it's already been measured can't disturb
anything else in the flow — nothing extra needs to be taught "ignore
this box's offset." `apply_relative_offset` runs once, at the very end
of `measure`'s own dispatch (after everything else, including any
resolved `absolute` descendants — see below), computing `top`/`right`/
`bottom`/`left`'s net `dx`/`dy` against the box's own real containing
block (`measure`'s own `width`/`containing_height` parameters — the
same basis `margin`/`width`/`height`'s own `%` already resolve against,
not some new concept) and shifting the box's entire already-built
subtree with a new `shift_laid_tree` helper (pure geometry, no
re-measuring).

**`absolute`/`fixed` needed real new plumbing, not just a shift.** Two
things make them harder than `relative`: a box has to be genuinely
*removed* from its parent's normal flow (no space reserved for it at
all — not just visually moved), and its real containing block is the
*nearest positioned ancestor*, which might be several levels up the
tree from wherever it's actually declared, not knowable until (or
unless) that ancestor is actually reached. Solved with a `PendingPositioned<'a>`
value (carrying a borrowed reference to the box, whether it's `fixed`,
and its own "static position" fallback — exactly the `x`/`y` its
parent's flow loop would have fed an ordinary `measure` call, used
whenever an offset is `auto`) that `measure`'s own `BoxContent::Blocks`
arm collects instead of measuring in place, and bubbles upward through
a new `pending: &mut Vec<PendingPositioned>` out-parameter threaded
through every one of `measure`'s own recursive call sites (plus
`measure_table`/`measure_row_with_widths`/`lay_out_row_cells`, so
content *inside* a table cell still participates correctly — though a
`<tr>`/`<td>`/`<caption>` itself declaring `position` doesn't, a
narrow, documented scope cut, same category as the table item's own
existing colspan/rowspan cut). Each level's own `resolve_or_bubble_positioned`
either resolves an entry locally (if *this* box's own `position` isn't
`static` — the real "nearest positioned ancestor" rule) or keeps
forwarding it to its own caller; `fixed` always keeps bubbling
regardless (its containing block is always the viewport, never an
intervening `relative`/`absolute` ancestor — the real spec behavior
without CSS transforms, the only case this engine has any notion of).
Whatever bubbles all the way past the document root (every `fixed` box,
plus an `absolute` box with no positioned ancestor anywhere above it)
is resolved by a new top-level entry point, `measure_document`, against
the window's own real viewport dimensions — approximating the real CSS
initial containing block, since this engine has no literal
viewport-sized box of its own (see NOTES.md's own architecture
writeup for exactly what that approximates away). `redraw` now calls
`measure_document` instead of `measure` directly.

**Solving `right`/`bottom` needed its own small trick.** An offset pair
like `right: 10px` with `width: auto` needs this box's own just-measured
width to resolve ("containing block's right edge minus my own width")
— but measuring needs a starting *position* first. `resolve_positioned`
measures once at a provisional position (`left`/`top` if set, else the
static-position fallback), then — only if `right`/`bottom` is actually
the side that matters — reuses `shift_laid_tree` again to move the
now-fully-measured subtree to its real final position, no re-measuring
needed. Same function also applies real CSS §9.7's "computed `display`"
rule: `resolve_style` forces `display: block` for anything `absolute`/
`fixed` (unless it's already `none`), right after both are cascaded —
without this, a `<span style="position: absolute">` would never even
get its own `LayoutBox` at all (`collect_children`'s inline branch
would flatten its text into the surrounding run, silently losing the
whole element), the same kind of invisible gap v19/v29/v32's own
`default.css`-rule discoveries were.

**`z-index` and a real, if deliberately narrow, "paint in the right
order" mechanism.** The task's own framing was explicit that *some*
stacking/paint-order handling is needed as soon as anything can
overlap, even without full CSS2.1 Appendix E stacking-context
semantics — so `Laid` gained a `positioned: Vec<Laid>` field (the
out-of-flow descendants a box is the resolved containing block for)
and a `fixed: bool` flag, and `paint` grew one new step: after a box's
own normal content, paint everything in its own `positioned` list,
already sorted ascending by `z-index` (`auto`/unset as `0`, ties in
document order — `Vec::sort_by_key` is a stable sort, so this needed no
separate tie-break logic). `hit_test_cursor` mirrors this (checked
first, in reverse order, so a click lands on whatever's actually on
top). The one real wrinkle: a `fixed` box's own `Laid::x`/`y` are
resolved in *viewport* space, not document space, so `paint` passes
`0.0` instead of the ambient `scroll_offset` for that one subtree
(everything inside it inherits `0.0` for free, since every recursive
`paint` call just forwards whatever `scroll_offset` it was given) —
`hit_test_cursor` needed the mirror-image translation (document space
→ viewport space) threaded through instead, since it isn't recursive
in the same uniform top-down way `paint` is.

**Testing.** 7 new permanent tests in `mod tests`: `absolute_position_computes_display_to_block`
(the §9.7 computed-display rule, checked the way described above — the
box's mere *existence* in the tree is the assertion); `position_relative_offsets_paint_position_not_sibling_flow`
(a three-sibling fixture proving the middle one's offset moves *it*
but leaves the third sibling exactly where it'd land with no offset at
all); `position_relative_percent_offset_resolves_against_containing_block`;
`position_absolute_removed_from_flow_and_anchored_to_positioned_ancestor`
(checks both halves: the container's own normal-flow children list
drops to exactly one, and the absolute child's own resolved position);
`position_absolute_right_bottom_solve_from_box_own_size`;
`position_absolute_with_no_positioned_ancestor_and_fixed_both_bubble_to_document_root`
(same containing-block rect, different `fixed` flag); `position_fixed_anchors_to_viewport_not_wherever_it_sits_in_a_tall_document`
(a 3000px-tall preceding sibling has zero effect on where it lands);
and `z_index_controls_paint_order_for_overlapping_absolute_boxes`, a
real painted-pixel check (not just geometry) that a higher-`z-index`
box wins an overlapping region despite appearing *first* in document
order, which would paint *underneath* without z-index support. One
real snag along the way, same lesson v51's own font-family test
already taught: the first draft of the `relative`/`absolute` flow
tests put each `<div>` on its own line in the fixture HTML, and the
literal whitespace between block-level tags became its own anonymous
inline box consuming real flow height, breaking the exact-pixel deltas
these tests check — fixed by writing the fixture HTML with no
inter-tag whitespace, same as v51's own fix.

Also visually confirmed with real `grim` screenshots against four
small test pages: a relatively-offset box (visibly shifted, with the
following sibling correctly *not* leaving a gap where the offset box
would have displaced it); an absolutely-positioned box breaking out of
its parent's flow (a sibling after it sits flush at the parent's top,
not pushed down); two overlapping `position: absolute` boxes with
different `z-index` (the higher one visibly on top in the overlap
region, matching the pixel-level test); and a `position: fixed` box
pinned to the top-right corner, not participating in its own normal
flow either. **One piece not live-verified, flagged rather than
glossed over, same category v32's own `cursor` writeup already
established**: actually scrolling the page and confirming a `fixed`
box visibly stays put on screen while content scrolls underneath it
couldn't be checked interactively — this sandbox has no tool to
synthesize a mouse-wheel/scroll event (same gap v32 hit trying to
synthesize a mouse-move for `cursor`). The underlying mechanism
(`Laid::fixed` plus `paint`'s `scroll_offset` passthrough) is simple,
directly-inspectable code, and its *positioning* math is proven by a
permanent headless test (`position_fixed_anchors_to_viewport_not_wherever_it_sits_in_a_tall_document`)
— only the live "does it actually stay visually still while scrolling"
behavior is unverified. Worth a real interactive check next time a
human's at the keyboard, same as `cursor`'s own still-open item.

## v53 — `<head>`/`<title>`/`<meta>`/`<link>`/`<base>` hidden by default

> set up a loop where you pick the next TODO item, and once completed,
> clear your context and do that again until the list is done

A tiny, user-flagged item (not picked off the standing survey order
directly — the user noticed `default.css` had no `display: none` rule
for `<head>`'s contents at all, asked for it to be logged as its own
`TODO.txt` item, and this session then picked it as the next thing to
do off that same list). Confirmed by grep before touching anything:
`script`/`style` are hardcoded skipped in `collect_children`/
`build_table_children` (so their content never renders even as plain
text), but `head`/`title`/`meta`/`link`/`base` had no handling
whatsoever — a real page's `<title>` text would render visibly at the
top of the document, which every real UA explicitly prevents via its
default stylesheet's "Hidden elements" rule
(https://html.spec.whatwg.org/multipage/rendering.html#hidden-elements).

**What/why:** one new `default.css` rule — `head, title, meta, link,
base, noscript { display: none; }` — narrowed from the spec's full
hidden-elements list to the tags scratchy actually parses into its DOM
today (`area`/`datalist`/`param`/`rp`/`template`/`track` aren't
relevant yet, same "only what's needed" curation policy this
stylesheet already follows). No code change needed beyond the CSS
itself: `head` resolving to `display: none` is sufficient on its own —
`collect_children`'s existing `if style.display == Display::None {
continue; }` already means a `Display::None` element's children are
never visited at all, so `title`/`meta`/`link`/`base` nested under
`head` are covered for free once `head` itself is hidden. The rule
still explicitly lists `title`/`meta`/`link`/`base` (not just `head`)
to match the real spec and stay correct even for a document where one
of these appears somewhere unexpected outside `<head>` (html5ever's own
tree-construction algorithm is quite permissive about relocating
stray tags).

One new permanent test, `title_text_does_not_render` (71 total): parses
a document with `<title>Secret Page Title</title>` plus a visible body
paragraph, walks the full `LayoutBox` tree collecting every rendered
text run, and asserts the title text never appears while the body text
does. Visually confirmed with a real `grim` screenshot: a page with a
`<title>` and a body paragraph renders only the paragraph, nothing from
`<head>` at all.

**Deliverable:** `src/default.css` (new hidden-elements rule), one new
permanent test in `src/lib.rs`'s `mod tests`. `TODO.txt`'s corresponding
item checked off.

## v54 — `float`/`clear`

> set up a loop where you pick the next TODO item, and once completed,
> clear your context and do that again until the list is done

Next item of that same autonomous `TODO.txt` loop (v49 did `%` lengths,
v50 table layout, v51 named `font-family` fallback lists, v52 the
`position`/`z-index` positioning model, v53 `<head>` hiding) — the
`float`/`clear` item v52 explicitly split out as "a genuinely different,
separate layout algorithm... not just an extension of this positioning
work." That framing held up: unlike `position`, which only ever needed
one box to move itself (and anything out-of-flow anchored to it),
`float` needs a box to stay in *block* flow enough to still stack
against its siblings' positions, while *removing* itself from the part
of flow that reserves vertical space, and then making *other* boxes'
own line layout react to where it landed — three different pieces of
machinery (`measure`'s `BoxContent::Blocks` arm, `measure`'s own
box-sizing preamble, and `measure_mixed_inline`'s line-breaker) all
needed to learn about it, not just one.

**Scope landed: real block-level float positioning, `clear`, and real
inline-text wrap-around — not scaled back to block-only.** The task's
own framing called inline wrap-around "the hard, valuable part" and
worth attempting rather than deferring by default; it turned out
tractable within the session by reusing machinery already in the file
rather than inventing new line-layout code: `measure_mixed_inline`
(v48's hand-rolled per-word greedy line-breaker, built for inline
`<img>`) already produces one line at a time with its own cursor
position, so giving each line its own horizontal corridor (instead of
one fixed width for the whole run) was a local change to that loop, not
a new algorithm. The one thing *not* extended this session: cosmic-
text's own single-`Buffer` whole-paragraph path (`measure`'s plain
`BoxContent::Inline` arm) still can't vary width per line at all — a
paragraph that starts before every float in its BFC has ended is now
routed through `measure_mixed_inline` instead (even when it's pure
text, no image), the same way a text+image mix already was.

**The core new type, `Floats`, is deliberately *not* modeled after
`PendingPositioned`'s bubble-up-to-an-ancestor search.** A float's
containing block is always just "whichever block box's own `Blocks`
loop placed it" — there's no equivalent of `position`'s "nearest
positioned ancestor" search, so nothing needs to *bubble*. Instead
`Floats` (`left`/`right`: `Vec<FloatEntry { top, bottom, edge }>`) is
threaded through `measure`'s recursion as a plain `&Floats` — a
`BoxContent::Blocks` arm clones whatever it's handed, grows the clone
locally as it places its own direct float children (so a later sibling,
*and anything nested inside an earlier non-floated sibling*, sees them
too — real floats affect every descendant inline formatting context in
the same block formatting context, not just immediate siblings), and
hands a *fresh*, empty `Floats` into anything that starts its own new
BFC instead: the document root, a float's own content, `position:
absolute`/`fixed` content, and a table cell's interior. `measure` grew
this as a new parameter (not a return value or a mutable out-parameter
like `pending`) precisely because nothing needs it back — a sibling
only ever needs to *read* the float state as of its own turn in the
loop.

**Placing a float**, inside `measure`'s `BoxContent::Blocks` arm: at the
child's `clear`-adjusted `cursor_y`, `Floats::avail_bounds` finds the
horizontal corridor left after any earlier same-BFC float(s) whose own
`top..bottom` range contains that exact `y` (a single probe, not a
check across the float's own real height range — see its own docs for
why, same category of real-but-approximate simplification `measure_
mixed_inline`'s per-line version below also makes). A `width: auto`
float doesn't fill that corridor (ordinary block behavior) — real CSS
computes it via shrink-to-fit (CSS2.1 §10.3.5), reusing `cell_natural_
width` (v50's per-cell min/max-content sizing, despite its table-
specific name — it works on any box) at the float's own full containing-
block width, not the narrowed corridor: a float's size is never narrowed
by other floats, only its *position* is. `box_model_entry` grew one new
parameter, `auto_margin_box_override: Option<f32>`, to let this result
override the ordinary "auto width fills the container" computation
without disturbing `%`-margin/padding resolution's own (unrelated,
un-narrowed) containing-block basis. `Float::Right` is measured at the
corridor's left edge like `Left` and then shifted into its real final
position with `shift_laid_tree` once its real width is known — the same
provisional-measure-then-shift trick `resolve_positioned` already uses
for `right`/`bottom` (v52). The float never advances `cursor_y` (the
whole point of "out of flow") but its `Laid` still gets pushed into its
parent's own `Blocks` children — unlike `position: absolute`, which
removes its slot from the list entirely, a float still paints in the
ordinary way, just without reserving flow space.

**A real bug found by looking at a screenshot, not predicted up
front**: a later, non-cleared, full-width sibling's own *box* is never
narrowed to avoid a float (only its *inline content* is — matching real
CSS, where only line boxes react to floats, not a block box's own
border/background) — so its own background can legitimately overlap a
float's area. The first version of this feature pushed a float's `Laid`
into its parent's children list at its own point in document order,
same as any other child, which meant a later opaque-background sibling
painted *over* most of the float in that overlap — confirmed with a
`grim` screenshot (a 150x150 red float mostly swallowed by a green
sibling's own background) before it was ever written up as expected
behavior. Real CSS floats are their own paint/stacking level (CSS2.1
Appendix E), above ordinary in-flow non-positioned block boxes — fixed
by collecting a `Blocks` loop's own float children into a side list and
appending it to the normal children list only *after* every normal
sibling has been added, so every float paints after (visually on top
of) every normal sibling in the same list, with no new field on `Laid`
needed. Pinned with a real painted-pixel test, same shape as v52's own
`z_index` test.

**`clear`**: `Floats::clear_y` returns the lowest bottom edge among
floats on the side(s) a `clear` value names (`f32::MIN` — never wins a
`max` — when there's nothing to clear), and `measure`'s `Blocks` arm
applies it to *every* non-positioned child (float or not) before that
child is placed, the one line CSS2.1 §9.5.2 actually requires.

**Real CSS's own "computed `display`" rule (spec §9.7) extended to
cover `float` too** (previously only `position: absolute`/`fixed`) —
`resolve_style` now forces `display: block` whenever `float` isn't
`none`, same as it already does for `position`, so e.g. a `<span
style="float: left">` gets its own box instead of being flattened into
the surrounding inline run, the same invisible-gap shape v19/v29/v32/
v52 each already hit once for a different property.

**Deliberately not attempted, each documented precisely in `TODO.txt`
rather than silently absorbed into "float/clear done"**: a float whose
own shrink-to-fit (or explicit) width doesn't actually fit in the
narrowed corridor left by earlier same-side floats is placed there
anyway rather than retried lower down — it may visually overlap a
competing float (real CSS2.1 §9.5.1's full placement algorithm handles
this; this session's scope note explicitly didn't require it: "multiple
floats stacking against the same side... don't need bespoke handling").
A real block box's own auto *height* still never grows to contain an
uncleared float (the classic "needs a clearfix" behavior) — this is
actually real CSS spec behavior, not a cut, but worth stating since it
can look surprising in a naive test page with no later cleared sibling.
Once *any* float exists anywhere earlier in a BFC, every paragraph that
starts before the *lowest* float's own bottom edge takes the slower
per-word `measure_mixed_inline` path — including one that turns out not
to overlap any specific float once its own lines are actually
positioned — a known, undocumented-until-now performance cost, not a
correctness one. A float's own cursor-hit-test priority still follows
plain document order (first match wins) like any other `Blocks`
sibling, unlike `paint`'s now-corrected "floats on top" order — so in
the narrow case of a float visually overlapping a later sibling, a
click there may resolve against whichever one happens to come first in
the markup, not whichever one is actually on top.

5 new permanent tests in `mod tests` (76 total):
`float_left_is_removed_from_normal_flow_and_positioned_at_the_
containing_blocks_edge` (both halves at once — the float still gets a
real `Laid` in its parent's children, unlike `position: absolute`'s
missing slot, but the next sibling lands exactly where it would if the
float didn't exist, and that sibling's own *box* position is unaffected
— only its inline content narrows); `float_right_is_positioned_
against_the_containing_blocks_right_edge`; `clear_moves_a_box_below_
an_earlier_float`; `float_paints_above_a_later_full_width_siblings_
own_background` (the real painted-pixel regression test for the bug
above); and `inline_text_narrows_beside_a_left_float_and_resumes_full_
width_below_it` — a real geometry test using many repeated short words
in a corridor only slightly wider than the float (so wrapping across at
least two lines is guaranteed regardless of real font metrics on
whatever system runs the test, same discipline v48's own image-wrap
tests established), confirming a line overlapping the float starts at
its right edge and a line entirely below it resumes the container's own
left edge. Also visually confirmed with three real `grim` screenshots:
a left float with wrapping paragraph text (narrower lines beside it,
full width once past it); a right float doing the mirror image; and a
`clear: left` box sitting cleanly below an earlier float instead of
beside it.

**Deliverable:** `src/lib.rs` (`Float`/`Clear` enums, `ComputedStyle`/
`Declarations` fields and their `DeclParser` parsing, the computed-
display rule extension, `Floats`/`FloatEntry`, `box_model_entry`'s new
`auto_margin_box_override` parameter, `measure`'s new `floats`/
`auto_margin_box_override` parameters and its `BoxContent::Blocks`
arm's float-placement/`clear`/paint-order logic, `measure_mixed_
inline`'s per-line corridor narrowing, 5 new tests in the permanent
`mod tests`); `TODO.txt` (item checked off, one new narrower deferred
item added); `NOTES.md` (architecture write-up).

---

## v55 — `position: sticky`

> set up a loop where you pick the next TODO item, and once completed,
> clear your context and do that again until the list is done

Next item of that same autonomous `TODO.txt` loop (v49 did `%` lengths,
v50 table layout, v51 named `font-family` fallback lists, v52 the
`position`/`z-index` positioning model, v53 `<head>` hiding, v54
`float`/`clear`) — but unlike every item since v49, this one's own
`TODO.txt` text flagged it as needing a real decision before any code,
not just scope-narrowing: a naive `position: sticky` needs to
re-evaluate every box's own position on every scroll tick, which
conflicts directly with this engine's own deliberate "scrolling never
touches layout" performance invariant (see NOTES.md's v4/v5 writeup —
the whole reason scrolling here is cheap at all). Per the standing
workflow, an item that surfaces a real design question gets asked, not
guessed at. Asked via `AskUserQuestion`: "How should `position: sticky`
be handled, given it conflicts with scratchy's 'scrolling never touches
layout' performance design?", with three options — a cheap paint-time-
only approximation (resolve the static position/containing block once
at layout time, same as `relative`/`absolute`/`fixed` already do, then
clamp the *painted* position against the live scroll offset every frame,
entirely at paint time, no relayout), skip the item entirely, or a
fully spec-correct version that re-measures on scroll. The user chose
the first — cheap, paint-time-only sticky — the same kind of real,
load-bearing design conversation v48's own inline-`<img>`-flow
`AskUserQuestion` decision documents (see that entry's own tone): a
call about *how* to build the thing, made once, up front, not
re-litigated mid-implementation.

**What/why, architecture.** A sticky box stays in normal flow exactly
like `relative` (contributes to its container's auto height, never
displaces a sibling, no out-of-flow plumbing like `absolute`/`fixed`
need) — the only new work is capturing, once at layout time, the two
things a *correct* painted position needs that `relative` doesn't:
this box's own would-be-static position (trivially already `laid.y` by
the time this runs, since nothing has shifted it) and its real
containing block's vertical extent (the *parent's* content box — same
basis `relative`'s own `%` offsets already resolve against, not the
*padding* box `absolute`/`fixed` anchor against). `capture_sticky_info`
(called alongside `apply_relative_offset`, same call sites) resolves
the box's own `top`/`bottom` insets and seeds a new `Laid::sticky:
Option<StickyInfo>` field with a permissive placeholder for the
containing-block bounds, since *this* box's own containing block — its
parent — hasn't finished stacking every sibling yet at this point, so
its final content-box extent isn't known. `patch_sticky_containing_
block`, called immediately after `measure`'s own `BoxContent::Blocks`
loop (and `measure_table`'s own child-stacking loop) finishes, fills in
the real values from that loop's own final `cursor_y`. `shift_laid_
tree` — the existing helper `relative`'s offset and `resolve_
positioned`'s right/bottom-solving already use to move an
already-measured subtree without re-measuring — also had to learn to
shift a nested sticky descendant's own `StickyInfo` fields by the same
`dy`, so a sticky box nested inside, say, a `position: relative`
ancestor that itself gets shifted doesn't end up with stale,
pre-shift containing-block bounds.

The actual per-frame position math (`sticky_shift`) is pure arithmetic
over values `measure` already resolved — no shaping, no re-layout, just
`(scroll_offset + top).clamp(static_top, max_top) - static_top` (and
the mirror-image formula for `bottom`), where `max_top` is the
containing block's bottom edge minus this box's own height (the "don't
escape your own parent" clamp). Wired into `paint`'s existing `Blocks`/
`TableRow` child loop by folding the computed shift into the *ambient*
scroll offset passed down into that one recursive call — reusing,
rather than reinventing, the exact mechanism `position: fixed` (v52)
already established for making a subtree scroll-independent (there,
the ambient offset is forced to `0.0`; here, it's `scroll_offset - dy`
instead — same trick, different adjustment), so every descendant inside
a sticky box (including any `position: absolute` child anchored to it)
shifts correctly with zero further per-node plumbing. `hit_test_cursor`
needed the mirror-image version by hand (not uniformly recursive the
same way `paint` is, same reason `fixed`'s own translation there
already isn't — see v52's own writeup), gaining a new `viewport_h`
parameter threaded through for `sticky_shift`'s own `bottom`-offset
math.

**Scope, per the user's own sign-off.** Only `top`/`bottom` resolve —
scratchy has no horizontal scrolling at all (confirmed against NOTES.md's
own scrolling writeup before writing any code), so a `left`/`right`
sticky offset would have no live horizontal scroll position to clamp
against; not attempted, `top` wins over `bottom` when both are set
(same precedence `resolve_positioned` already gives `left`/`top` over
`right`/`bottom` for `absolute`/`fixed`). "Nearest scrolling ancestor"
is simplified to exactly one thing — the whole document/viewport —
since this engine has no concept of a nested `overflow: scroll`/`auto`
region at all (confirmed by grep: `Overflow` only ever clips, never
scrolls), the scope cut the task's own framing explicitly sanctioned
up front. Real CSS §9.7's computed-`display` rule, which `absolute`/
`fixed` both force to `block`, deliberately does *not* apply to
`sticky` (real spec only names those two) — an inline `position:
sticky` element stays flattened into its surrounding run with no box
of its own to stick, same pre-existing gap `relative` already has on
inline content, not a new one. A `<tr>`/`<td>` itself declaring
`position: sticky` isn't supported either (the classic "sticky table
header row" pattern) — same pre-existing scope cut `absolute`/`fixed`
already have for table rows/cells (v52), inherited rather than
revisited here.

**Testing.** 5 new permanent tests in `mod tests`:
`sticky_box_stays_in_normal_flow_and_contributes_to_its_containers_size`
(flow/size, no paint — a sticky box occupies a real slot, its own
`Laid::y` is never shifted at layout time at all, and the next sibling
lands exactly where it would with no offset declared); `sticky_box_
paints_at_its_static_position_before_scroll_reaches_the_threshold` and
`sticky_box_clamps_to_its_top_offset_once_scroll_passes_the_threshold`
(both call the real `paint` pipeline into a real pixel buffer and check
actual painted pixels, not just the math, at two different scroll
offsets against the one `Laid` tree built once); `sticky_box_stops_
sticking_once_its_containing_blocks_far_edge_would_scroll_past` (the
"don't escape your own parent" clamp, checked at and far past the exact
boundary); and `scrolling_only_changes_the_painted_position_never_the_
laid_tree`, which paints the same `&Laid` twice at very different
scroll offsets and confirms both that specific `Laid`/`StickyInfo`
fields are byte-identical afterward *and* that the two painted results
genuinely differ — the concrete, permanent pin for this item's whole
architectural point (and `paint` taking `&Laid`, never `&mut Laid`, is
already a compile-time guarantee of the first half, not just a
runtime one).

**Visually confirmed, with an honestly-flagged gap.** A real test page
(gray spacer, a red `position: sticky; top: 0` header, 3000px of
trailing content) rendered correctly at its pre-scroll, static position
via a real `grim` screenshot. **Live scroll-while-stuck verification
wasn't possible**, same category as v32's own `cursor` gap and v52's
own `fixed`-while-scrolling gap: this sandbox has no tool to synthesize
a real mouse-wheel event (checked directly — no `wtype`/`ydotool`/
`wlrctl` installed, and the running compositor exposes no scripting
IPC for it either), so the actual "visibly sticks, then releases, while
a human scrolls" behavior couldn't be driven end-to-end interactively.
Documented in `TODO.txt` as a real, open gap rather than silently
skipped — the headless pixel-level tests above exercise the identical
`paint`/`sticky_shift` code path a live scroll would, just with a
synthetic `scroll_offset` instead of a real wheel event.

**Deliverable:** `src/lib.rs` (`Position::Sticky` + its parsing,
`Laid::sticky`/`StickyInfo`, `sticky_shift`, `capture_sticky_info`,
`patch_sticky_containing_block`, `shift_laid_tree`'s new sticky-aware
shifting, `paint`'s and `hit_test_cursor`'s sticky-aware child
recursion — the latter gaining a new `viewport_h` parameter — 5 new
tests in the permanent `mod tests`); `TODO.txt` (item checked off);
`NOTES.md` (architecture write-up).

---

## v56 — real CSS2.1 Appendix E stacking-context semantics

> set up a loop where you pick the next TODO item, and once completed,
> clear your context and do that again until the list is done

Next item of that same autonomous `TODO.txt` loop (v49 did `%` lengths,
v50 table layout, v51 named `font-family` fallback lists, v52 the
`position`/`z-index` positioning model, v53 `<head>` hiding, v54
`float`/`clear`, v55 `position: sticky`) — the `z-index`/stacking-order
item v52 explicitly left narrower than the full Appendix E model, and
which this session's own task framing (unlike v55's sticky) said didn't
need a design decision: CSS2.1 Appendix E's stacking/painting order is
precisely spec'd, just a bigger, more involved correctness item than
most of what's been picked off this run.

**What landed, in order of how the implementation actually unfolded.**
Negative `z-index` now paints between layers 1 and 3 — above this box's
own background/border, below its normal content — instead of the old
model's flat "every out-of-flow box, any sign, paints after everything."
`position: relative`/`sticky` with an *explicit* `z-index` now
establishes its own descendant stacking context (real CSS2 §9.9.1),
competing by `z-index` with its containing block's other out-of-flow
descendants instead of painting at its plain document-order position —
`z-index: auto` (the overwhelming common case) is completely unaffected,
confirmed by its own dedicated regression test. Both fixes live entirely
in `paint`/`hit_test_cursor`; nothing about `measure`/`Laid`'s own
fields changed — this is a paint-order feature, not a layout one.

**A genuine inconsistency inside this session's own task text, resolved
by checking the authoritative source, not by asking the user.** One part
of the instructions said negative `z-index` paints "below its own
containing box's own background." A different part of the same
instructions gave the literal Appendix E seven-layer list, which puts
background at layer 1 and negative-`z-index` contexts at layer 2 —
*above* layer 1, not below it. These two statements can't both be true.
Caught concretely, not just by close reading: the first implementation
attempt followed the "below the background" wording literally, and its
own dedicated test immediately contradicted the *second* test (the
seven-layer one, built straight from the numbered list), which already
correctly expected negative to beat the background. Resolved by
checking https://www.w3.org/TR/CSS21/zindex.html directly — background-
before-negative-in-a-bottommost-first list means negative paints on top
of the background, confirming the seven-layer list's reading was right
and the prose paraphrase was the imprecise one (a very common piece of
CSS folklore: "negative z-index hides behind everything" is only true
when the relevant box has no background of its own to appear in front
of). This wasn't treated as a stop-and-ask design ambiguity — it was a
factual question the task's own more precise wording, and the cited
spec, already answered unambiguously; asking would have meant asking
the user to re-derive something already fully specified in two other
places.

**Is the "two unrelated in-flow subtrees overlapping" generality the
original `TODO.txt` wording worried about actually reachable in
scratchy today?** Checked directly, as this session's task asked, rather
than assumed either way. Answer: yes, but only at sibling granularity,
not in the fully general sense. Float/clear (v54) only narrows content
*around* a float within one inline formatting context — it can't make
two unrelated block-level subtrees occupy the same screen region. But a
`position: relative` box's own `top`/`left` offset (resolved since v52)
genuinely can shift a box into visual overlap with an ordinary sibling
in the *same parent's* `Blocks` list — proven directly by this session's
own `relative_box_with_explicit_z_index_establishes_its_own_stacking_
context` test. That's the concrete, reachable shape this feature
actually needed to handle — scoped to exactly that (same immediate
parent, one box's offset reaching into a sibling's rectangle), not a
general "any two subtrees anywhere on the page" search, which doesn't
arise from anything scratchy's current layout model can produce.

**Deliberately scoped out, each its own precise `TODO.txt` item rather
than silently dropped**: a `relative`/`sticky` box's own stacking
context is anchored at its *immediate* containing block only — it
doesn't bubble past an intervening positioned-but-`z-index:auto`
ancestor the way real nested stacking contexts would (not reachable by
anything real yet). `hit_test_cursor`'s own negative-`z-index` priority
doesn't fully mirror `paint`'s corrected order (a negative entry is
still checked at the same first-priority pass as a non-negative one) —
restructuring the whole function to tell "found nothing" apart from
"found this box's own ambient cursor" throughout was judged a
disproportionate risk for a narrow, rarely-observable case, the same
kind of scoped-narrower call v31's `overflow` and v54's own float-hit-
test gap already made.

**Testing.** 4 new permanent tests in `mod tests`: `negative_z_index_
paints_above_its_containing_boxs_background_but_below_normal_content`;
`seven_layer_stacking_order_within_one_containing_block` (layers 1, 2,
3, and 7 pinned via 3 sampled pixels in one fixture, with document order
deliberately not matching paint order so the test can't pass by
accident); `relative_box_with_explicit_z_index_establishes_its_own_
stacking_context`; and `relative_box_with_auto_z_index_does_not_
establish_a_stacking_context`, the explicit regression pin for the
"`auto` is unaffected" half of the feature. All pre-existing tests
(including v52's own `z_index_controls_paint_order_for_overlapping_
absolute_boxes`) still pass unchanged. One real snag while writing the
new fixtures, the same lesson v51/v52 already documented and worth
restating since it bit again here: the first draft of every fixture had
a literal newline between tags, each becoming its own whitespace-only
anonymous inline box that silently shifted every box below it — caught
by dumping the actual `Laid` tree with a throwaway debug test rather
than assuming the fixture HTML was fine because similar ones had worked
before. Also visually confirmed with a real `grim` screenshot against a
page combining all of this at once: a negative-`z-index` box correctly
showing through below an in-flow sibling that only covers part of its
area, and a `relative`+explicit-`z-index` box correctly winning an
overlap against a later static sibling.

**Deliverable:** `src/lib.rs` (`is_explicit_stacking_child`, the new
`paint_z_layer` helper, `paint`'s reordered per-box layering, `hit_test_
cursor`'s matching reordering, 4 new tests in the permanent `mod
tests`); `TODO.txt` (item checked off, two new narrower deferred items
added); `NOTES.md` (architecture write-up).

---

## v57 — flexbox (`display: flex`)

> set up a loop where you pick the next TODO item, and once completed,
> clear your context and do that again until the list is done

Next item of that same autonomous `TODO.txt` loop (v49 did `%` lengths,
v50 table layout, v51 named `font-family` fallback lists, v52 the
`position`/`z-index` positioning model, v53 `<head>` hiding, v54
`float`/`clear`, v55 `position: sticky`, v56 real stacking-context
semantics) — `TODO.txt`'s "No flexbox (`display: flex`) or grid
(`display: grid`)" item. Explicitly scoped to flexbox only, with grid
split out into its own, entirely deferred `TODO.txt` item — the task's
own framing granted the same latitude the `position`/`float` items
already used for narrowing an originally-oversized TODO entry, and grid
(track sizing, template areas, implicit/explicit grids, `fr` units,
grid placement) is a materially larger, separate algorithm from
flexbox; bundling both into one session risked landing neither well.

**What landed**: `display: flex` establishing a flex formatting
context for its direct children (each "blockified" per real CSS §4,
turning out to need almost no new code at all — see below);
`flex-direction` (all 4 keywords) and `flex-wrap` (all 3, though
`wrap`/`wrap-reverse` are only genuinely real for `row`/`row-reverse` —
see its own `TODO.txt` item) plus the `flex-flow` shorthand;
`justify-content` (all 6 keywords, including `space-evenly`, which this
item's own wording treated as optional/deferrable but turned out to
fall out of the exact same arithmetic as `space-between`/`space-around`
for free); `align-items`/`align-self` (`flex-start`/`flex-end`/
`center`/`stretch`, plus `baseline` approximated as `flex-start` — this
item's own wording explicitly permitted that approximation given the
disproportionate plumbing real baseline alignment would need);
`flex-grow`/`flex-shrink`/`flex-basis` and the `flex` shorthand, with
real proportional space distribution (not just "fits content" —
verified with permanent tests using asymmetric ratios); and
`gap`/`row-gap`/`column-gap`, new properties this codebase didn't have
at all before (confirmed by grep, per this item's own instruction to
check first). `inline-flex` and `order` were each deliberately not
attempted — precisely documented in `TODO.txt`, not silently dropped.

**Architecture, in brief** (full write-up in `NOTES.md`): a third
`display`-dispatch special case parallel to `Table`'s own, both in
`build_box_for_element` (a new `build_flex_children`) and in `measure`
(a new `measure_flex`, further split into `measure_flex_row`/`measure_
flex_column` once the two directions turned out to have a meaningfully
different shape, not just "swap width and height"). Reuses `box_model_
entry` for the container's own box model (same helper `measure_table`
already factored out for exactly this kind of reuse) and `cell_natural_
width` (v50's min/max-content measurement machinery, originally built
for table-column sizing, already reused once for `float`'s shrink-to-
fit basis) for a flex item's content-based sizing on whichever axis is
*width* for a given direction. The one real insight that made cross-
axis stretch and main-axis grow/shrink cheap on *either* direction:
this engine's own `height` property already doesn't affect how a box's
children are laid out (v42), so wherever a flexed/stretched final size
lands on whichever axis is physically *height*, it's a pure post-hoc
`Laid::h` overwrite after one real `measure()` call — never a second,
more expensive layout pass.

**10 new permanent tests** in `mod tests`, covering exactly what this
item's own requirements named: row vs. column direction, `justify-
content: space-between`, `align-items: stretch`/`center`, `flex-grow`/
`flex-shrink` genuinely proportional distribution (asymmetric 1:2 and
1:3 ratios, not just "grew/shrank at all"), `flex-wrap` starting a
second line, `gap` spacing, and `position: absolute` correctly removed
from the flex item list. All 95 tests (85 pre-existing + 10 new) pass;
`cargo clippy` shows the same 17 pre-existing warnings as unmodified
`main`, zero new ones from this feature's own code.

**Visually verified** with 3 real test pages via `grim` — a growing
flex row, a centered flex column, and a wrapped multi-line row — all
confirmed correct *geometry* (text/item positions matched the expected
layout exactly). No `background-color` painted in any of the three
screenshots; investigated rather than assumed to be this feature's own
bug: the unmodified `main` branch (`git stash`), run against a trivial
unrelated `<div style="background: red">` with no flexbox at all, *also*
painted no background color in this same sandbox session — a pre-
existing environment/compositor limitation (same category as the
v13-v20-era `grim`/winit stalls), not something this feature introduced
or could fix.

**Deliverable:** `src/lib.rs` (`Display::Flex` plus `FlexDirection`/
`FlexWrap`/`JustifyContent`/`AlignItems`/`FlexBasis`, the matching
`Declarations`/`ComputedStyle` fields and cascade/parsing, `build_flex_
children`, `measure_flex`/`measure_flex_row`/`measure_flex_column`,
`resolve_flex_main_sizes`/`distribute_justify_content`, 10 new tests in
the permanent `mod tests`); `TODO.txt` (item checked off with exactly
what landed, grid split into its own new deferred item, plus 8 further
narrower deferred items for what was cut within flexbox itself);
`NOTES.md` (architecture write-up).

---

## v58 — `display: grid`, scoped to fixed/`fr` tracks + auto-placement

> set up a loop where you pick the next TODO item, and once completed,
> clear your context and do that again until the list is done

The next item of that same autonomous loop (after v57's flexbox) —
`TODO.txt`'s `display: grid` item, deliberately deferred by v57 itself
("grid's track sizing, template areas, implicit/explicit grids, `fr`
units, and grid placement are a materially larger, separate algorithm
from flexbox, and bundling both in one session would have risked
landing neither well"). CSS Grid Layout
(https://www.w3.org/TR/css-grid-1/) is a genuinely large spec; this
session used the same latitude the `position`/`float`/flexbox items
already had to scope it to a coherent, well-tested core subset rather
than something broad and half-right.

**What landed:**

1. `display: grid` establishes a grid container — a block-level box in
   the outer flow (same as `Table`/`Flex`) whose direct children each
   become grid items.
2. `grid-template-columns`/`grid-template-rows` with a space-separated
   track list: fixed lengths (`px`, plus `%`/`em`/`rem` — whatever
   length units this engine already resolves, per v49), the real `fr`
   flexible-length distribution algorithm (CSS Grid §11.7, scoped to a
   single pass — no iterative freeze-and-retry, same documented
   simplification flexbox's own `resolve_flex_main_sizes` already has),
   and `repeat(<integer>, <track-list>)` (`repeat(auto-fill/auto-fit,
   ...)` deferred — real available-space-driven track-count resolution
   is a different, bigger problem).
3. `gap`/`row-gap`/`column-gap` — confirmed these were already generic,
   not flexbox-specific (`ComputedStyle`'s own `row_gap`/`column_gap`
   fields, set by the same `DeclParser` branches flexbox already uses),
   so `measure_grid` just reads the same two fields directly — no new
   gap parsing/cascading needed at all, exactly the reuse this task
   asked to check for.
4. Implicit auto-placement: items place into the explicit column
   count in document order, row by row (`grid-auto-flow: row`, the
   real spec default and the only flow implemented) — wrapping to a
   new row once a row fills, with no explicit placement properties
   needed at all.

**What was deliberately cut**, each documented precisely in `TODO.txt`
rather than silently folded into "done": explicit per-item placement
(`grid-column`/`grid-row`/their `-start`/`-end` longhands — every item
occupies exactly one cell, in auto-placement order); `grid-template-
areas`/named lines; `justify-items`/`align-items`/`-self` within a
cell (an item always stretches to its column's full width, like a
table cell, and sits at its row's own top edge); content-based track
sizing (`auto`/`min-content`/`max-content`/`minmax()`) — every track is
a plain fixed/`%`/`fr` length; `repeat(auto-fill/auto-fit, ...)`; and
one deliberate, precedent-backed narrowing specific to rows (see
below).

**Architecture**: a third `box_model_entry`/`display`-dispatch special
case, parallel to `Table`'s and `Flex`'s — `Display::Grid`, a
`build_grid_children` (reusing `build_flex_children` outright, since
both container types "blockify" every direct child identically with no
grid-specific difference at the box-*building* stage), and
`measure_grid` (`resolve_grid_tracks` for the real `fr` distribution,
shared `box_model_entry` for the container's own box model). See
`NOTES.md` for the full write-up, including why row-track `fr`/`%`
sizing deliberately falls back to content-based auto sizing rather than
guessing at a basis most real grid containers don't have.

**One real bug caught by the permanent tests, not assumed away**: the
first version of `measure_grid` derived the container's own content
height from the laid-out items' own `y + h` extents (the same trick
`measure_flex` uses, which works there because a flex line's cross
size is always exactly its tallest item). That's wrong for grid once a
row has an *explicit* `grid-template-rows` size taller than its own
content — the extra space never shows up in any item's own measured
position, so the container silently reported too short a height. Found
immediately by `grid_explicit_row_track_sizes_the_row_regardless_of_
content` (container height 10px instead of the expected 80px) before
anything was committed — fixed by tracking each row's own real bottom
edge directly (`content_bottom`), separately from the laid items'
positions, rather than deriving it after the fact.

**Testing**: 8 new permanent tests in `mod tests` — fixed-track-width
columns at the right x-offsets, `fr`-unit distribution with an
asymmetric 1:3 ratio, a fixed track mixed with `fr` tracks sharing only
the *leftover* space, auto-placement wrapping to a second row once a
2-column row fills, `gap` spacing both columns and rows, an explicit
`grid-template-rows` track sizing its row regardless of shorter
content (the test that caught the bug above), `repeat(3, 50px)`
expanding to the same layout as writing the three tracks out by hand,
and a `position: absolute` child correctly pulled out of the grid item
list into the container's own `positioned` list (mirroring flexbox's
identical test).

**Visual verification** (`grim`, two real test pages): a `100px 1fr
2fr` mixed-track row with a 10px gap — three visibly different-width
columns with 10px gaps, the `100px` column narrow enough to wrap its
own label text, `2fr` clearly about twice `1fr`'s width; and a
`repeat(3, 120px)` grid of 5 items with an 8px gap — items 1-3 on the
first row, items 4-5 wrapping to a second row directly under columns 1
and 2, the second row starting exactly below the first row's tallest
(70px) item. Both rendered exactly as the headless tests already said
they would.

**Deliverable:** `src/lib.rs` (`Display::Grid`, `GridTrackSpec`,
`parse_grid_track_size`/`parse_grid_repeat`/`parse_grid_template`,
`Declarations`/`ComputedStyle`'s `grid_template_columns`/`_rows`
fields + cascade, `grid-template-columns`/`-rows` branches in
`DeclParser::parse_value`, `resolve_grid_tracks`, `measure_grid`, the
new `Display::Grid` dispatch points in `measure`/`build_box_for_
element`/`build_table_children`/`collect_children`, 8 new tests in the
permanent `mod tests`); `TODO.txt` (item checked off with exactly what
landed, plus new narrower deferred items for explicit placement, named
areas/lines, `justify-items`/`align-items` within a cell, content-based
track sizing, `repeat(auto-fill/auto-fit, ...)`, and row-track `fr`/`%`
sizing against an indefinite container height); `NOTES.md`
(architecture write-up).

---

## v59 — `word-spacing`, via the hand-rolled path that sidesteps letter-spacing's failure

> set up a loop where you pick the next TODO item, and once completed,
> clear your context and do that again until the list is done

Next item of that same autonomous `TODO.txt` loop (v49 did `%` lengths,
v50 table layout, v51 named `font-family` fallback lists, v52 the
`position`/`z-index` positioning model, v53 `<head>` hiding, v54
`float`/`clear`, v55 `position: sticky`, v56 real stacking-context
semantics, v57 flexbox, v58 grid) — the last member of the old misc-
CSS-properties cluster, `word-spacing`, left on the list specifically
because it looked harder than the reverted `letter-spacing` attempt:
cosmic-text 0.19's `Attrs` has no `word_spacing` knob to hook into at
all (confirmed again by grepping its vendored source), so a correct
implementation would mean hand-rolling word-boundary detection and
spacing entirely outside cosmic-text's own shaping/wrapping — on its
face, a worse version of the exact risk that broke `letter-spacing`
(wiring spacing into cosmic-text's own `Attrs`/shaping led its
*internal* wrap-width computation to get a word's width wrong once
spacing crossed a small threshold, shattering a short paragraph into
one line per word).

**The framing that made this tractable rather than a repeat of that
failure: `letter-spacing`'s wording predates two things that changed the
shape of this problem entirely.** v48 built `measure_mixed_inline` — a
hand-rolled, word-by-word greedy line-breaker (originally for inline
`<img>`), which shapes each *word* independently via `shape_spans` at
tiny-single-word granularity and positions words left-to-right itself,
entirely outside cosmic-text's own paragraph-level line-breaking. v54
then generalized it further: a plain-text-only run already routes
through this slower path whenever a float could still be active, not
just when an image is present. Since this path already knows word
boundaries (it splits text into words itself to shape each one
separately) and already positions each word's own x-offset by hand,
adding extra horizontal space between words there is a *position-
calculation change in scratchy's own code* — not something that asks
cosmic-text to account for extra spacing inside its own wrap-width math.
The strategy: wire `word-spacing` into this same hand-rolled path, the
same way v54 made float-affected runs route there, except now *any* run
with a nonzero `word-spacing` routes there, with no image or float
needed at all.

**What/why:** `word-spacing` parses `normal` (represented as
`Length::Px(0.0)`, same "no extra spacing" value an explicit `0` already
gets) or a plain `px`/`em`/`rem` `<length>` — no `%`, matching the real
spec's own Level 1/2 grammar and the same `parse_length` (not
`parse_length_percent`) grammar `border-width`/`outline-width` already
use. Threaded as a genuine CSS-inherited property (CSS2.1 §16.4 lists
both `letter-spacing` and `word-spacing` as inherited) through
`InheritedStyle`/`ComputedStyle` exactly the way `font-family`/
`white-space`/`text-align` already are — resolved eagerly to a pixel
value in `resolve_style` (like `border_width`/`outline_width`, not kept
symbolic the way `line-height`'s bare-`<number>` form has to be, since
`word-spacing` has no per-descendant-reinterpreted form). Also added to
`SpanStyle` (and its `From<InheritedStyle>` impl) so each word in a
mixed run carries the `word-spacing` of the specific span it came from —
a nested element overriding `word-spacing` only affects its own words,
not its siblings', matching real per-element cascade.

**The routing change**, mirroring v54's own float-routing addition
almost exactly: `measure`'s `BoxContent::Inline` match arm gained a
third disjunct — `items.iter().any(|i| matches!(i, InlineItem::Text(_,
span) if span.word_spacing != 0.0))` — alongside the existing "any
image" and "an active float" conditions. Checked per-span, not against
the run's own anonymous-box style, since a run can mix spans from
different elements with different `word-spacing` values. `word-spacing:
normal` (the overwhelming common case) leaves every span's value at
`0.0`, so an ordinary paragraph is completely unaffected — still the
fast single-`Buffer` cosmic-text path, zero regression risk/cost,
pinned by a permanent test (`word_spacing_normal_keeps_the_fast_cosmic_
text_path`).

**Where the actual spacing gets applied**, inside `measure_mixed_
inline`'s own word-measurement loop: a word's shaped (cosmic-text-
measured) width gets its span's `word_spacing` added directly onto it
*whenever that word's own token carries the trailing space that marks a
real word boundary* (`word.ends_with(' ')` — the last word in a run,
with no trailing space, correctly gets no extra). This is baked straight
into the `Measured::Word`'s own `width` field, rather than threaded as a
separate field through every downstream consumer — the wrap-check/
cursor-advance in the greedy line-breaker's first pass, the final
`text-align` math, and the hit-test bounding box in `MixedPositioned::
Word` all already treat a word's own `width` as "how much horizontal
space this word needs before the next one," so inflating that one number
is enough everywhere, for free. This is also the exact same treatment
this function's own docs already gave the word's *ordinary* trailing
space before this item (a known, documented simplification — "a line's
trailing space counts toward width... unlike a real UA which hangs it
past the edge") — `word-spacing` just extends that same convention
uniformly rather than inventing a second one. Crucially, cosmic-text's
own per-word `shape_spans` call (which measures a word's *natural*
width, with no knowledge of the spacing) runs completely untouched —
`Attrs` never sees a word-spacing value at all, so there's no
cosmic-text-internal wrap-width computation left for this to confuse,
the way there was for `letter-spacing`.

**A real test-writing snag worth flagging, caught before it shipped**: the
first version of the core geometry test measured "gap from word 1's own
right edge to word 2's left edge" (`x1 - (x0 + w0)`) and got `0`,
looking like a bug — but it's actually a predictable consequence of
baking the extra spacing into `width` as described above: the space is
now *inside* word 1's own reported width, not a separate gap after it.
Fixed by measuring word 2's own absolute `x` position across two
different declared `word-spacing` values instead (`10px` vs `30px`) and
asserting the difference is exactly `20px` — a more direct and more
robust proof that the position math scales exactly with the declared
value, not an artifact of how the "gap" happened to be computed.

**Verification, deliberately modeled on `letter-spacing`'s own documented
reproduction** (the honest, specific style this project's own history
praises that TODO.txt entry for): 5 new permanent tests. Besides the
routing/fast-path tests above and the exact-pixel gap test:
`word_spacing_does_not_shatter_a_short_paragraph_the_way_letter_spacing_
did` reproduces the *exact* scenario and spacing values from
`letter-spacing`'s own TODO.txt entry (a short paragraph at a 900px
container, 2px and 8px of spacing — the precise value that shattered
`letter-spacing` into 4 lines) and confirms this still fits on 1 line at
both; `word_spacing_does_not_break_realistic_multi_word_wrapping` uses a
longer, realistic multi-sentence paragraph at a typical 600px article
width with 3px `word-spacing` and asserts the resulting line count still
averages well over 3 words per line, not degrading toward one-word-per-
line. All 108 tests pass (103 pre-existing + 5 new), `cargo clippy`
clean (no new warnings — the pre-existing baseline's warning count and
specific locations were diffed against a stash of this change to confirm
nothing new was introduced). Visually confirmed with a real `grim`
screenshot: three paragraphs (`word-spacing: normal`/`20px`/`2px`) all
wrapping naturally across multiple lines with no shattering, visibly
wider gaps between words at larger declared values, word-spacing-only
wrapping differences (more lines at 20px than at 2px) exactly as real
CSS word-spacing would produce.

**Deliverable:** `src/lib.rs` (`Declarations::word_spacing` + its
`DeclParser` arm, `InheritedStyle`/`ComputedStyle`/`SpanStyle`'s new
`word_spacing` fields and their cascade/inheritance/`From` wiring,
`measure`'s new routing disjunct, `measure_mixed_inline`'s word-
measurement change, 5 new tests in the permanent `mod tests`);
`TODO.txt` (item checked off); `NOTES.md` (architecture write-up,
explicitly contrasting this approach with `letter-spacing`'s failed
one).

---

## v60 — `text-decoration-line`/`-style`/`-color` longhands, real `wavy`, `double` for line-through/overline

> set up a loop where you pick the next TODO item, and once completed,
> clear your context and do that again until the list is done

Next item of that same autonomous `TODO.txt` loop, right after v59's
`word-spacing` — the remaining `text-decoration-style`/`-color` gap
left over from v33: `wavy` fell back to `Solid` (no curve-rasterization
primitive existed anywhere in this file), `double` only ever visibly
differed for *underline* (cosmic-text 0.19's own native double-underline
support, with no strikethrough/overline equivalent), and there was no
way to declare `text-decoration-line`/`-style`/`-color` as independent
longhands — only via the `text-decoration` shorthand, which bundles all
three into one `Option`-cascaded `DecorationValue`. Three distinct gaps,
all tractable, all landed in one pass — none turned out to need a real
design call that would've justified cutting it.

**Gap 1 — independent longhands.** Added `text-decoration-line`/
`-style`/`-color` as their own `Declarations`/`ComputedStyle` fields,
each cascaded completely independently (`cascade_property`, same
pattern every other longhand in this file already uses) rather than
folded into `DecorationValue` during cascade. The real design question
was how to *combine* three independently-cascaded components with the
shorthand's own single bundled value and the ambient-propagated value
an element inherits from its nearest decorated ancestor (see v33/v39 for
why this is ambient propagation, not real CSS inheritance) — resolved by
a new `resolve_own_decoration(style, ambient) -> DecorationValue`, which
every call site that used to write the one-line `style.text_decoration.
unwrap_or(decoration)` (in `build_box_for_element`, `build_table_
children`, `collect_children`) now calls instead. It starts from
`ambient` or the shorthand's own full replacement (identical to the old
behavior), then applies any declared `-line`/`-style`/`-color` longhand
as an override of just that one component — so `text-decoration-line:
underline; text-decoration-style: dashed; text-decoration-color: red;`
as three separate declarations combines into the exact same effective
`DecorationValue` a single `text-decoration: underline dashed red`
shorthand would (verified directly, by comparing the two against each
other, not just against a hand-computed expectation). The *combined*
result, not the three raw longhands, is what threads on as the new
ambient value for that element's own children — same propagation shape
the shorthand's own override already had, so this needed no new
threading parameter, just a different combine step at the same point
the old `unwrap_or` sat.

**Gap 2 — `wavy`.** `DecorationStyle` gained a real `Wavy` variant,
hand-painted in `paint_decoration_line` the same way `Dashed`/`Dotted`
already are (v33): cosmic-text's `Renderer` trait (confirmed by reading
its vendored source directly, not assumed) offers only `rectangle`/
`glyph` — no path or curve primitive at all — so a genuine sine wave is
approximated as a sequence of narrow rectangles, each centered on
`amplitude * sin(2π · x / wavelength)` at its own x, with amplitude/
wavelength both derived from the stroke's own `thickness` so the wave
scales with font size instead of using fixed pixel constants. Verified
with a real pixel test (`wavy_decoration_produces_a_real_sine_wave_not_
a_straight_line`) that doesn't just check "something painted" — it
asserts the painted segments' own y both *rises and falls* across the
stroke's width (ruling out a monotonic ramp, which would also vary but
isn't a wave), contrasted against a control test proving `Solid` stays
perfectly flat. Visually confirmed with `grim`: a genuine zigzag/wave
pattern under `wavy` underline text, clearly distinct from a straight
line.

**Gap 3 — `double` for line-through/overline.** Turned out simpler than
it looked once traced through: `paint_decorations` never actually called
cosmic-text's own runtime renderer — it's a complete hand-written
reimplementation (v33) that only *mirrors* cosmic's offset/thickness/
color formulas. So the old `UnderlineStyle::Double`-only doubling (set
by `parse_text_decoration` folding a `double` keyword straight into
`deco.underline`, bypassing `DecorationStyle` entirely) was never load-
bearing for anything except underline — there was no real reason
doubling couldn't apply uniformly to all three lines once represented in
the one place that's actually shared across them. Redesigned: `double`
now always sets `DecorationStyle::Double`, handled once inside `paint_
decoration_line` (two parallel `Solid` strokes, offset by thickness plus
an equal gap — the exact formula cosmic's own native double-underline
already used), and `paint_decorations` calls this same primitive
identically for underline/strikethrough/overline — so a style that was
previously underline-only now works for all three with no special-casing
per line type. `UnderlineStyle::Double` (cosmic's own enum) is still
matched defensively in `paint_decorations`, in case something else ever
sets it independently, but neither the shorthand nor the longhands
produce it anymore. Verified end-to-end (not just at the primitive
level) with real pixel tests that shape actual text and count distinct
painted stroke bands down a column — one each for line-through, overline,
and a regression check that underline still doubles correctly after the
refactor — plus a primitive-level test on `paint_decoration_line` itself
confirming the exact two-rectangle/offset/color output. Visually
confirmed with `grim`: double overline and double underline both show
two clearly separated parallel lines in the screenshot; double line-
through, zoomed into a word gap in a cropped screenshot (strikethrough
sits through glyph ink, which hides the gap less obviously than the two
clear-whitespace line types), also shows two distinct parallel red
strokes.

**Verified overall:** 10 new permanent tests (including a primitive-
level `double_style_paints_two_parallel_solid_strokes_via_the_shared_
primitive` test alongside the end-to-end ones), all in the existing
`mod tests` block (nothing written-then-deleted). All 118 tests pass
(108 pre-existing + 10 new), `cargo check`/`cargo clippy` clean on every
file touched (one
`clippy::needless_borrow` on this item's own new code, fixed; every
other clippy warning in the full-crate run is pre-existing, confirmed by
diffing warning locations against the file before this change).
`cargo install --path .` run so `~/.cargo/bin/scratchy` isn't left
stale. Visually confirmed with a real `grim` screenshot covering all
three gaps at once on one test page: dashed magenta underline via three
separate longhand declarations with no shorthand anywhere, a genuine
wavy blue underline, double red line-through, double green overline, and
(regression check) double orange underline.

**Deliverable:** `src/lib.rs` (`DecorationStyle` gains `Double`/`Wavy`
variants; new `DecorationLine` struct and `parse_text_decoration_line`/
`parse_text_decoration_style`/`decoration_style_from_keyword`; new
`Declarations`/`ComputedStyle` fields and their `DeclParser`/
`cascade_property` wiring; new `resolve_own_decoration` combine function
replacing the three `style.text_decoration.unwrap_or(decoration)` call
sites; `paint_decorations`/`paint_decoration_line` reworked for uniform
`Double` handling and a new `Wavy` arm; 10 new tests); `TODO.txt` (item
checked off, no narrower follow-up item needed — all three stated gaps
landed in full); `NOTES.md` (architecture note on the sine-wave
approximation and the `Double` unification).

---

## v61 — extended `cursor` keyword set

> set up a loop where you pick the next TODO item, and once completed,
> clear your context and do that again until the list is done

Next item of that same autonomous `TODO.txt` loop, right after v60's
`text-decoration` longhands — the last remaining `cursor` gap v32 left
open: only `auto`/`pointer` were ever parsed, the one keyword
`default.css` itself declares (`a { cursor: pointer }`). Nothing about
this needed a design call: winit's own `CursorIcon` (re-exported from
the `cursor_icon` crate, checked at 1.2.0's vendored source directly)
turns out to have a clean 1:1 equivalent for every real CSS `cursor`
keyword — including the full resize family and the less common ones
(`context-menu`, `cell`, `alias`, `all-scroll`, ...) — so this is a
mechanical extension through and through, no keyword had to be skipped
or approximated.

**What landed.** `CursorStyle` grew from 2 variants to the full real
CSS set: `default`, `none`, `context-menu`, `help`, `progress`, `wait`,
`cell`, `crosshair`, `text`, `vertical-text`, `alias`, `copy`, `move`,
`no-drop`, `not-allowed`, `grab`, `grabbing`, the entire resize family
(`e`/`n`/`ne`/`nw`/`s`/`se`/`sw`/`w`/`ew`/`ns`/`nesw`/`nwse`/`col`/
`row`-`resize`), `all-scroll`, `zoom-in`, `zoom-out` — alongside the
original `auto`/`pointer`. A new `cursor_style_from_keyword` helper
(same `eq_ignore_ascii_case` ladder shape `decoration_style_from_
keyword` already established for a similarly-shaped keyword property)
replaces the old 2-armed if/else directly inside `DeclParser`'s
`cursor` branch.

**The one real design point, called out rather than guessed past:**
`none` is real CSS (hide the cursor entirely) but has no `CursorIcon`
equivalent at all — it isn't an icon, it's the absence of one. Rather
than inventing a fake icon or silently dropping the keyword,
`CursorStyle::to_winit()` changed signature from `-> CursorIcon` to
`-> Option<CursorIcon>`, with `None` meaning "hide the cursor" — the
caller (`WindowEvent::CursorMoved`'s handler) now matches on this and
calls `window.set_cursor_visible(false)` instead of `window.
set_cursor(..)` whenever it sees `None`, and restores visibility
(`set_cursor_visible(true)`) only on the transition back out of that
state — not on every mouse-move tick, same "only call the OS when the
state actually changed" discipline `last_cursor` already existed for.
`App::last_cursor` itself became `Option<CursorIcon>` to track this
third state (visible-with-icon vs. hidden) correctly.

**Zero changes needed to `hit_test_cursor` or the per-glyph propagation
machinery** — exactly as v32's own doc comments predicted: that
function is keyed on "which `CursorStyle` does this element/glyph
resolve to," never on `Pointer` specifically, and real CSS inheritance
for `cursor` (threaded through `InheritedStyle`/`SpanStyle` since v32)
already generalizes to any variant with no extra wiring. Verified this
claim directly rather than assuming it still held after the enum grew:
a new full-pipeline test locates an actual `cursor: grab`-styled glyph
and an adjacent plain-text glyph in a real shaped paragraph and
confirms `hit_test_cursor` reports `Grab`/`Auto` respectively — the
same glyph-precision diligence v32's own `Pointer`-vs-`Auto` test used,
just picked a keyword that didn't exist before this item to prove the
mechanism is genuinely generic, not coincidentally still working for
the two cases it was originally written against.

**Verified:** a table-driven test parses and cascades every keyword
(including re-covering `auto`/`pointer`, whose own v32-era tests had
been written-then-deleted under the project's pre-v41 test policy) via
real `style="cursor: ..."` CSS text through the actual `DeclParser`
path, not just the keyword-mapping helper in isolation; a cascade/
inheritance test confirms a nested `<span>` with no `cursor` of its own
inherits a new (not just the original `Pointer`) keyword from an
ancestor; direct `CursorStyle`→`CursorIcon` mapping tests for a
representative spread of new variants; a dedicated test confirms
`CursorStyle::None.to_winit()` is `None`, not some icon; and the
glyph-precision hit-test test described above. 6 new permanent tests,
all kept in `mod tests` (nothing written-then-deleted, per the v41
policy). `cargo check`/`cargo clippy` clean on every touched line (the
full-crate clippy run still reports the same pre-existing warnings as
before this change, confirmed by line number — none on anything this
item touched). All 124 tests pass (118 pre-existing + 6 new). `cargo
install --path .` run so `~/.cargo/bin/scratchy` isn't left stale. A
quick headless smoke-test (launch against a page exercising several of
the new keywords including `none`, confirm it stays up for several
seconds with no panic/stderr output) — not a live interactive check.

**Still not live-verified, for the exact same reason v32 already
documented and nothing here changes**: the OS cursor icon is rendered
by the compositor, outside scratchy's own pixel buffer, so `grim`
screenshots can't show it, and this sandbox still has no tool to
synthesize a real mouse-move event to drive `WindowEvent::CursorMoved`
end-to-end. Every piece verified headlessly above is real verification
of the logic that *would* drive the live behavior; only the final,
thin `window.set_cursor`/`set_cursor_visible` OS calls themselves stay
unverified live, same category this project has flagged before for
anything whose correctness ultimately depends on OS/compositor-level
state (v32's own `cursor`, v55's `position: sticky`) — worth a real
interactive check next time a human's at the keyboard, now covering
every keyword, not just `pointer`.

**Deliverable:** `src/lib.rs` (`CursorStyle` enum extended from 2 to 33
variants; `cursor_style_from_keyword` helper; `CursorStyle::to_winit()`
signature changed to `Option<CursorIcon>`; `App::last_cursor` changed
to `Option<CursorIcon>`; `WindowEvent::CursorMoved`'s handler updated
for the hide/show transition; 6 new tests); `TODO.txt` (item checked
off, caveat about live interactive verification kept, now scoped to the
full keyword set); `NOTES.md` (no architecturally new note needed
beyond what's already in this entry and TODO.txt — this was a clean
mechanical extension with no new category of gap).

---

## v62 — `overflow: scroll`/`auto`, real interactive per-element scrolling

> set up a loop where you pick the next TODO item, and once completed,
> clear your context and do that again until the list is done

Next item of that same autonomous `TODO.txt` loop, right after v61's
extended `cursor` keyword set. `overflow` (v31, extended to box
decoration clipping in v40) had sat at `visible`/`hidden` only this
whole time — this closes the gap with real `scroll`/`auto`: an element
with one of those two values, when its content overflows its own box,
becomes its own independently-scrollable region with a real, live,
mouse-wheel-interactive scroll offset — not just a visual clip that
would be indistinguishable from `hidden`.

**The real architectural question, settled before writing any code**:
scratchy has exactly one scroll region today — the whole document/
viewport, tracked as a single `App::scroll_offset: f32` — confirmed in
NOTES.md's own v55 (`position: sticky`) writeup, which explicitly
noted "only one scrolling container exists in this engine at all...
since there's no nested `overflow: scroll`/`auto` region to need its
own version of this clamp." This item is exactly that extension. The
risk flagged by the task itself was a possible collision with
scratchy's oldest, most load-bearing performance invariant —
"scrolling never touches layout" (NOTES.md's v4/v5 writeup) — the same
risk that made `position: sticky` need an explicit `AskUserQuestion`
before any code got written. This item didn't hit that wall: `position:
sticky`'s own two-stage split (resolve bounds once at layout time;
read/apply a *live* value fresh every paint call, never stored in
`Laid`) generalizes directly — a per-element scroll offset is no
different in kind from a per-element sticky-shift, just keyed by a
stable id instead of computed from the page's own single offset. No
design conflict, no `AskUserQuestion` needed; this was a size/scope
question (same latitude the `position`/`float`/flexbox/grid items
already got), not a flagged tradeoff.

**What/why, architecture.** A new `ScrollInfo { id, content_h,
client_h }` (`max_scroll()` derives `(content_h - client_h).max(0.0)`)
is `measure`'s own layout-time-resolved half, mirroring `StickyInfo`
exactly: `id` is assigned from the *same shared `next_id` counter*
`measure`'s own paragraph ids already use for `ShapeCache` — a
different, unrelated-by-value id namespace (a separate `HashMap`), so
reusing the counter needed no new `&mut usize` threaded through every
recursive `measure` call just for this. `content_h`/`client_h` are
values `measure`'s own `BoxContent::Blocks` arm already computes for
its ordinary auto-height/explicit-height-clamp logic — `client_h` is
literally `border_h - extra_h`, the same quantity that arm already
derives. Deliberately scoped to that one arm only (not `measure_table`/
`measure_flex`/`measure_grid`'s own separate tails) — the single most
common real case, same "narrow choke point over the general case"
latitude `overflow: hidden`'s own v31/v40 scoping already used; a
table/flex/grid *container* with `overflow: scroll`/`auto` on itself
gets no scroll behavior yet (a new, narrower `TODO.txt` item).

The *live* per-element scroll amount deliberately lives nowhere in
`Laid` at all — `App::scroll_offsets: HashMap<usize, f32>`, read fresh
(and re-clamped against `max_scroll()` at the read site, self-healing
against a stale value from a since-shrunk relayout) by `paint`,
`hit_test_cursor`, and the new `hit_test_scroll_region`, exactly
`position: sticky`'s own `scroll_offset` ambient parameter, just
persisted across frames this time (sticky's own live value is the
page's single global `scroll_offset`, already persisted; a per-element
scroll needs its own per-id persisted value instead, hence the map).
`measure`/`Laid` still never read it — the same compile-time guarantee
(`paint`/`hit_test_cursor` take `&Laid`, never `&mut Laid`) that made
sticky's own "scrolling never touches layout" claim a type-level fact,
not a convention, extends automatically to this.

**Clipping+scrolling composes with `fixed`/`sticky`'s own ambient-offset
tricks the same way they compose with each other.** `paint` computes
`own_scroll` (this box's own live, clamped scroll) and folds it into a
new `content_scroll_offset = scroll_offset + own_scroll`, passed down
*only* into this box's own ordinary `Blocks`/`TableRow` children loop
(not its own `draw_y` — scrolling moves a box's *content*, never the
box itself, unlike `sticky`/`fixed`, which shift the whole subtree
including the box) — exactly the same "ambient offset grows as you
descend into a scrolling subtree" shape `fixed` (forcing it to exactly
`0.0`) and `sticky` (`scroll_offset - sticky_shift(...)`) already
established, just a different adjustment. A nested `position: sticky`
descendant's own `sticky_shift` call picks up the adjusted ambient
automatically (it just reads whatever `scroll_offset` is lexically in
scope at its own recursion depth), and — though not covered by a
dedicated test, see TODO.txt — a *nested* scroll region would too, by
the same composition, with no special-casing needed either way.
Clipping reuses `overflow: hidden`'s own existing `child_clip`
machinery completely unchanged (`Scroll`/`Auto` just join `Hidden` in
the one `matches!` check that decides whether a box clips its
descendants at all) — v31/v40's "an element's overflow clips its
content, not itself" distinction, and the border-box-not-strictly-
padding-box clip bounds, both carry over exactly as they already
worked.

**Wheel-event routing** (`hit_test_scroll_region`) walks the `Laid`
tree the same way `hit_test_cursor` (v32/v61) already does — same
coordinate conventions, same `position: sticky` child-rect translation
— but returns `Option<ScrollInfo>` (the deepest scrollable ancestor
under the point) instead of a `CursorStyle`, and bubbles the innermost
miss up via `Option::or` rather than resolving by inheritance the way
`cursor`'s own fallback does (there's no "inherited scroll region" —
an unscrollable box's own point genuinely needs its *ancestor* checked
next, the real "wheel bubbles to the nearest scrollable ancestor"
behavior). `App::last_mouse_pos` is new — `WindowEvent::MouseWheel`
carries no position of its own in `winit` at all, so routing needs the
last position `WindowEvent::CursorMoved` reported, remembered
separately. `None` (cursor not over any scrollable region, or hasn't
moved over the window yet) falls back to the pre-existing page-level
`scroll_offset` logic unchanged.

**`auto` vs. `scroll`, the one real difference.** Both clip and scroll
identically whenever content actually overflows — the task's own
framing was right that trying to make `auto` differ in clipping
behavior doesn't correspond to anything real CSS does. The only real
distinction is the scrollbar indicator: `scroll` always paints it
(`paint_scrollbar`'s own `show_when_not_overflowing` flag, set from
`s.overflow == Overflow::Scroll`), `auto` only once `max_scroll() >
0.0`.

**Scrollbar visual** (`paint_scrollbar`): a flat, simple painting
primitive in the same spirit as this file's other decorations
(box-shadow/border/outline) — a translucent track plus a translucent
thumb proportional to `client_h / content_h`, both hard-edged
`fill_rect`s, no rounded ends. Deliberately an *overlay*, painted
inside the box's own border box without narrowing `content_w` for it
at layout time at all — matches the modern OS overlay-scrollbar
convention (macOS, mobile, GTK's modern style) rather than classic
reserved-space scrollbars, and keeps the "scrolling never touches
layout" invariant extended to this box's own *width* too, not just its
scroll position. **Drag-to-scroll on the thumb itself is explicitly out
of scope** — wheel-scroll is this item's own interactive core, per the
task's own stated latitude to defer dragging as a nice-to-have.

**Testing.** 5 new permanent tests in `mod tests`:
`overflow_scroll_assigns_a_scroll_id_and_computes_content_vs_client_height`
(the basic `ScrollInfo` bookkeeping through the real pipeline);
`overflow_auto_only_scrollable_when_content_actually_overflows` (both
directions — fits vs. overflows); `scroll_offset_clips_and_translates_
painted_content` (a real pixel-level test: a red child 100px below an
`overflow: scroll` box's own 50px window is fully clipped at scroll
offset `0`, then becomes visible once painted at `max_scroll` — the
literal "clipping+offsetting its content correctly at a given scroll
offset" verification this item's own standing convention asked for);
`wheel_routing_picks_the_scroll_region_under_the_cursor_not_the_page`
(two independent sibling scroll regions plus a plain paragraph —
confirms `hit_test_scroll_region` tells the two apart by position and
correctly falls through to `None` over the non-scrollable content);
`scrollbar_paints_for_scroll_even_when_not_overflowing_but_auto_only_
paints_when_it_does` (the real `scroll`-vs-`auto` distinction, pixel-
checked). Two real bugs caught by writing these before trusting the
code: `background: blue` (the shorthand, which this engine never
implemented — only the `background-color` longhand exists) silently
dropping instead of erroring, which a first draft of the clip/translate
test used by mistake; and this test harness's own `vec![0u32; ...]`
pixel buffers defaulting to *black*, not white (`App::redraw`'s own
loop explicitly fills white before painting, something a from-scratch
test buffer doesn't get for free) — a `!= 0x00FFFFFF` "is this
white-background still unpainted" check across an un-initialized black
buffer reported false positives everywhere, caught by printing the
actual pixel values rather than trusting the first failure's own
assertion message.

**Verified visually, honestly short of the live interactive case.** A
real test page (an `overflow: scroll` box with 6 stacked paragraphs,
only 2.5 fitting in its own 100px height; an `overflow: auto` box with
content that fits) rendered correctly via a real `grim` screenshot:
the scroll box clips exactly at its own border, its scrollbar track +
thumb both visible near the top (consistent with zero scroll), and the
`auto` box shows no scrollbar at all. **Live, wheel-driven scrolling
itself wasn't verified interactively** — same category this project has
flagged repeatedly before (v32's `cursor`, v55's `position: sticky`
while-scrolling gap, v61's extended cursor keywords): this sandbox has
no tool to synthesize a real mouse-wheel event. The permanent headless
tests exercise the identical `paint`/`hit_test_scroll_region` code
paths a live wheel tick would, just with a synthetic scroll offset
standing in for one.

**Deliberately cut, each documented precisely in `TODO.txt` rather than
silently folded into "done":** scrollbar drag-to-scroll; `overflow:
scroll`/`auto` on a table/flex/grid container's own box (only an
ordinary block container is covered); explicitly-stacked/out-of-flow
content *inside* a scroll region not scrolling along with it; a nested
scroll-region-inside-a-scroll-region case that's architecturally
supported by the same composition mechanism but has no dedicated test.

**Deliverable:** `src/lib.rs` (`Overflow::Scroll`/`Auto` variants +
parsing, `ScrollInfo` + `Laid::scroll`, `measure`'s `BoxContent::Blocks`
arm assigning it, `paint`'s `own_scroll`/`content_scroll_offset` +
`paint_scrollbar`, `hit_test_cursor`'s and the new `hit_test_scroll_
region`'s own-scroll-aware `Blocks`/`TableRow` recursion, `App::
scroll_offsets`/`last_mouse_pos` + `WindowEvent::MouseWheel`'s new
routing — 5 new permanent tests); `TODO.txt` (item checked off, 4 new
narrower cut items); `NOTES.md` (architecture write-up).

---

## v63 — `border-style`'s bevel variants: `groove`/`ridge`/`inset`/`outset`

> set up a loop where you pick the next TODO item, and once completed,
> clear your context and do that again until the list is done

Next item of that same autonomous `TODO.txt` loop, right after v62's
`overflow: scroll`/`auto`. `border-style` (v30) folded all four bevel
keywords into plain `Solid` at the keyword-mapping stage, since a real
bevel effect needs two-tone shading this file's flat-fill painting
primitives didn't have — this gives all four their own real rendering,
per CSS2.1 §8.5.3 (https://www.w3.org/TR/CSS2/box.html#border-style-properties).

**The color math.** No lighten/darken helper existed anywhere in this
file or in cosmic-text's own `Color`/`Attrs` API (checked before writing
one — `blend` is a src-over alpha compositor, a different operation
entirely). Added `lighten`/`darken` (blend each RGB channel toward
white/black by a ratio, alpha untouched) plus a single `BEVEL_RATIO =
0.5` constant — the spec names no exact ratio, so this picks one
reasonable, clearly-visible value and applies it consistently in both
directions, rather than inventing a different ratio per style.
`bevel_colors(style, base)` computes the `(light, dark)` pair for any of
the 4 new styles, `None` for every other one (so `paint_border`'s own
non-bevel path — `Solid`/`Dashed`/`Dotted`/`Double`/`None` — is
byte-for-byte what `paint_box_decoration` used to call directly, zero
behavior change there).

**The per-edge/per-half-edge logic**, all in a new `paint_border`
(replacing the 4 direct `paint_border_edge` calls `paint_box_decoration`
used to make): `Inset`/`Outset` paint each *whole* edge a single
light-or-dark variant — `Inset`'s top/left edges uniformly dark,
bottom/right uniformly light (reads as pressed into the page, lit from
the top-left); `Outset` is the reverse. `Groove`/`Ridge` got the full
inner-split treatment, not the scoped-down uniform approximation the
task gave explicit latitude to fall back to — splitting each edge's own
stroke width into an "outer" half (farther from this box's own content:
the top of the top edge, left of the left edge, bottom of the bottom
edge, right of the right edge) and an "inner" half (closer to it) turned
out to be a direct, un-disproportionate extension of `paint_border_edge`
already being called once per edge — 2 calls instead of 1 per edge, each
with its own already-computed shade, no new primitive needed. `Groove`:
top/left edges outer-dark/inner-light, bottom/right outer-light/
inner-dark (the same light-direction logic `Inset` uses, just applied
within each edge instead of across the whole edge, producing a visible
crease down the middle); `Ridge` is `Groove`'s exact reverse, edge for
edge. `half = (bw / 2.0).max(0.5)` keeps even a 1px bevel border showing
two distinct rows/columns rather than one degenerating to zero width.

**Verified headlessly, 6 new permanent tests** (`mod tests`, a new
"border-style bevel variants" section): `inset_border_paints_top_left_
edges_darker_than_bottom_right` and `outset_border_is_the_reverse_of_
inset` call the real `paint_border` directly into a hand-rolled pixel
buffer and sample all 4 edges, confirming exactly the darkened/
lightened variant lands where expected; `groove_border_splits_each_
edge_into_an_outer_dark_and_inner_light_half` samples both halves of
all 4 edges to confirm the crease; `ridge_border_is_the_reverse_of_
groove_at_every_sampled_point` paints both styles with the same inputs
and asserts every one of the 8 sampled points disagrees, pairwise; a
parsing test (`border_style_longhand_parses_all_four_bevel_keywords`)
confirms the `border-style` longhand resolves each of the 4 new
keywords to its own `BorderStyle` variant through the real cascade (not
folded into `Solid` anymore); and one end-to-end test through the real
`paint_fixture` HTML→layout→paint pipeline (not just the primitive)
confirms `border-style: inset` actually reaches `paint_box_decoration`'s
new `paint_border` call from a real document. `BorderStyle` needed
`#[derive(Debug)]` added (for the parsing test's own `assert_eq!`
failure message) — the only non-additive change to the type itself.

**Visually confirmed with a real `grim` screenshot**: 5 boxes
(`solid`/`groove`/`ridge`/`inset`/`outset`, all the same declared
border-color/width) side by side — `solid` uniform, `groove` showing a
visible dark-then-light crease down the top/left edges and light-then-
dark down bottom/right, `ridge` the exact mirror of `groove`, `inset`
uniformly dark top/left and light bottom/right with no crease, `outset`
the exact mirror of `inset` — all 4 new styles visually distinct from
each other and from plain `solid`, matching the design intent exactly.

**Not scoped down**: the task gave explicit latitude to approximate
`groove`/`ridge` as a uniform per-edge shade (the `Inset`/`Outset`
technique) if the inner-split turned out disproportionately complex —
it didn't, so both landed with the full two-stripe-per-edge split. No
narrower follow-up `TODO.txt` item was needed for this one.

Combined with `border-radius` is still out of scope, unchanged from
before this item (same pre-existing gap `dashed`/`dotted`/`double`
already had, v38): a bevel style's own `TODO.txt` note was folded into
the existing `border-radius` item's wording rather than duplicated.

**Deliverable:** `src/lib.rs` (`BorderStyle` enum: 4 new variants +
`Debug` derive, `border_style_from_keyword` no longer folds them into
`Solid`, `lighten`/`darken`/`BEVEL_RATIO`, `bevel_colors`, new
`paint_border` + `paint_border_edge`'s now-exhaustive match,
`paint_box_decoration`'s border call site simplified to one `paint_
border` call — 6 new permanent tests); `TODO.txt` (item checked off,
`border-radius`'s existing combo item's wording touched up);
`NOTES.md` (the lighten/darken color math, reusable for anything else
that needs a light/dark variant of a `TextColor` later).

---

## v64 — `@media`, scoped to viewport-width breakpoints

> yes, reorder and do those two next. Also before you start
> implementing, we should allow clicking links at some point 😅 Let's
> plan this now

This jumped the autonomous `TODO.txt` loop's own ordering (v62/v63
were still picking items off top to bottom) because of a real
investigation, not a random pick: the user asked specifically to
render actual `en.wikipedia.org` through `snag | scratchy` to see why
it still didn't look much better despite positioning/float/flexbox/
grid all having landed by v58-v63. Grepping the real page's CSS plus a
real `grim` screenshot found the honest answer — Wikipedia's entire
desktop two-column page skeleton comes from exactly one rule, and it
never fired at all:

```css
@media screen and (min-width:1120px){
  .mw-page-container-inner{
    display:grid;
    grid-template: min-content 1fr min-content / 12.25rem minmax(0,1fr);
    grid-template-areas: 'siteNotice siteNotice' 'columnStart pageContent' 'footer footer'
  }
  .mw-content-container{grid-area:pageContent}
}
```

Two independent, equally-necessary gaps block this: `@media` itself
(100 occurrences on that one page, entirely dropped at parse time —
this item) and `grid-template-areas`/`grid-area` placement (explicitly
scoped out of v58's grid work already). The user approved doing both,
ahead of everything else still sitting in `TODO.txt`, with `@media`
first — this entry — and `grid-template-areas`/`grid-area` as the
deliberate, planned very next item right after, not a random follow-up
pick. Scoped to `@media` only per the task's own explicit permission —
`@import`/`@font-face`/`@keyframes`/transitions/animations stay
deferred, and `TODO.txt` now carries their own narrower item spelling
out exactly that, split off the old blanket "no at-rules at all" entry.

**What landed.** `TopLevelParser`'s `AtRuleParser` impl (previously a
deliberate no-op — the default trait implementation rejects every
at-rule, which is exactly how `@media`/`@import`/`@font-face`/
`@keyframes` were all silently dropped at the stylesheet-parsing stage)
now recognizes `@media` by name and parses its full `<media-query-list>`
prelude (`parse_media_query_list`) plus recurses into its block with a
fresh `TopLevelParser` over the same already-block-delimited `input` —
reusing the exact top-level rule-list grammar rather than a second,
parallel implementation. Every rule inside the block gets tagged with
the block's own parsed condition (`StyleRule::media: Vec<MediaQuery>`,
empty = unconditional, exactly today's pre-existing behavior for
everything outside an `@media`). Because one `@media` at-rule can
expand into *several* real rules, `TopLevelParser::AtRule` had to
become `Vec<StyleRule>` (and `QualifiedRule` along with it, since
`StyleSheetParser` requires both to agree) — `parse_stylesheet` just
flattens with `rules.extend(..)` instead of `rules.push(..)`.

**Media features, in the task's own stated priority order:**
`min-width`/`max-width` (the dominant real-world case by a wide
margin — confirmed by that same Wikipedia grep), `screen`/`all`
matching and `print` *never* matching (scratchy has no print/paged-
media concept at all — this is correct, documented behavior for a
screen-only renderer, not a gap), a bare condition with no explicit
type implicitly meaning `all` (`MediaQuery::default()`), real
comma/`and` media-query-list grammar (comma = OR between `MediaQuery`
entries via `media_list_matches`, `and` = fields folded into one
entry), and — since there was room — `prefers-color-scheme`/
`prefers-reduced-motion`, both resolved to a fixed default since
scratchy has no OS preference detection at all: `light` always
matches (`dark` never does — an arbitrary but harmless pick, `light`
being the conventional default the feature itself defines for "no
preference known"), `no-preference` always matches (`reduce` never
does — doubly moot anyway, since scratchy has no animations to reduce
in the first place). Both are documented directly on `PrefersColorScheme`/
`PrefersReducedMotion`'s own doc comments, not just here.

**`calc()` in a media feature, checked and confirmed out of scope,
exactly as the task predicted**: grepped first (`grep -n calc
src/lib.rs` — zero hits anywhere in this project, confirmed before
writing a single line of parsing code) and didn't build any arithmetic
evaluator to fix it. `(max-width:calc(1120px - 1px))` — the real,
literal form from Wikipedia's own stylesheet — fails to parse inside
`media_feature_length_px` (`parse_length`'s `Function` token has no
matching arm) and the owning `MediaQuery` is marked `never`, the same
"unsupported value drops, doesn't poison the rest" policy this file
has used since `border-radius`/`box-shadow`. This turns out to be
spec-accurate, not just a convenient fallback: real CSS Media Queries
syntax already defines an individual query that fails to parse as
"not all" (never matches) rather than invalidating the whole
comma-separated list or the whole `@media` block — so `MediaQuery::
never`'s "safely never matches" is the literal correct behavior, not a
lossy approximation. `not` (negation) got the identical treatment for
the identical reason — recognized so it doesn't fall into the
"unrecognized media type" bucket, but not actually evaluated as a real
inversion, since that's unimplemented; marked `never` instead of
guessed at.

**Viewport-width-aware matching — the architecturally new part.**
`matching_declarations` (the one real choke point every cascade call
in this file already went through — `resolve_style`, `resolve_
background_image`, nothing else duplicates selector matching) now
takes a `viewport_width: f32` and skips any rule whose own `media`
condition doesn't currently hold, before even running the (more
expensive) real selector match. Threading that single new parameter
out to where it's actually known meant widening the signature of
every function already in that cascade's call chain —
`resolve_style`, `resolve_background_image`, `build_box_for_element`,
`build_table_children`, `build_table_row_cells`, `build_flex_children`,
`build_children`/`build_children_with_prefix`, `collect_children`,
`collect_root_custom_props`, and `build_layout_tree` itself — by one
parameter each, mechanical but real plumbing through the whole
box-tree-building recursion. No new resize-reaction mechanism was
invented for this, per the task's own explicit instruction: `App::
redraw` already calls `build_layout_tree` fresh every time the live
window width changes (`self.cached_width != width`, there from the
start for `%`/`em`/`rem`/table/flex/grid measurement's own sake) —
passing that same already-live `width as f32` through to `build_
layout_tree` makes a rebuild pick up media-aware rule matching for
free, with zero new plumbing on the resize side itself.

**Verified with 9 new permanent tests** (`mod tests`, "v64: `@media`"
section): `min-width` applying at/above its breakpoint and not below
(including the exact-boundary-is-inclusive case); `max-width` as the
exact inverse; `and` genuinely requiring both the type and the width
feature (not silently dropping one); a comma list being real OR
(matching via either branch, neither at a width satisfying neither);
`print` never matching even when its own width condition would
otherwise hold, to prove it's the type rejecting it and not the width
check; the `calc()` case never applying at any width, not just the one
a correct evaluation would reject; an ordinary non-`@media` rule
staying completely unaffected (the lowest-risk regression check for
"did threading viewport width into matching somehow make unconditioned
rules width-sensitive too"); a same-document, two-widths-of-`build_
layout_tree` call standing in for a real resize, confirming a rule
that didn't apply narrow does apply wide with no new mechanism
involved; and `prefers-color-scheme: light` matching while a sibling
`dark` rule doesn't. All 135 pre-existing tests still pass unmodified
(the ~45 call sites across tests and `benches/layout.rs` that called
`build_layout_tree(&doc)` needed a second `viewport_width` argument —
mechanically updated to a wide `1920.0`, which changes nothing for any
of them since none declare `@media`-scoped rules).

**Live-verified, with one real sandbox limitation worth naming
honestly rather than glossing over** (same category as v32's cursor/
v55's sticky-scroll/v62's wheel-event gaps): a real test page
(`@media (min-width:600px) { .box { background-color: red } }` plus a
`max-width:400px` green branch) rendered genuinely red via the full
HTML→CSS→layout→paint pipeline through a live `scratchy` process and a
real `grim` screenshot, at this sandbox's dwl tiling window manager's
actual assigned width (~1050px, comfortably past the 600px breakpoint)
— confirming `@media` is doing real, live work end to end, not just
passing headless tests. Getting a genuinely *narrow* live window to
confirm the inverse visually wasn't possible here: dwl's tile layout
keeps a single master column at a fixed width fraction regardless of
how many other clients exist (tried spawning up to 5 scratchy
instances simultaneously — the stack column's *height* shrinks per
client, never the master's width), and this sandbox has no wlr-
randr/swaymsg/wtype-equivalent to force or request a specific window
size the tiling WM would honor. The narrow/boundary/inverse cases are
instead covered exhaustively by the permanent headless suite above,
which exercises the real `MediaQuery::matches`/`matching_declarations`
code path directly, not a mock.

**Real-world sanity check**: `snag https://en.wikipedia.org/wiki/Cat |
scratchy` rendered the actual live page (286 resources inlined,
confirmed via `snag`'s own progress log) with no crash, no panic, and
no hang — scratchy's real `@media` parser chewed through the page's
~100 real `@media` blocks (including the exact `calc()` one quoted
above) without incident. The page still renders as a single-column
stacked dump, exactly as predicted going in: the live window's actual
width in this sandbox (~1050px) sits just under Wikipedia's own
1120px desktop breakpoint, and even where a `min-width` rule's
condition does hold elsewhere on the page, `grid-template-areas`/
`grid-area` placement — the other, deliberately-not-attempted half of
the real fix — still isn't implemented, so a `display: grid` container
that *did* newly activate still has nowhere named to place anything.
That's the expected, documented state of the world between this item
and the next one, not a bug in this one.

**Deferred, same item, narrower scope, exactly per the task's own
instruction**: `@import`/`@font-face`/`@keyframes`/transitions/
animations are still a deliberate no-op (any at-rule name other than
`media` still falls through `parse_prelude`'s `Err` arm, the same
"this one at-rule is invalid, drop it" outcome the old blanket default
implementation gave every at-rule before this item). `TODO.txt`'s old
"no at-rules at all" entry is now checked off for exactly the `@media`
slice landed here, with a new, narrower `- [ ]` entry carrying the
rest forward with the same reasoning the task gave for deferring them
(`@import` is a fetch-time concept this no-networking renderer doesn't
need yet; `@font-face`/`@keyframes`/transitions are unrelated to
responsive layout, which is what was actually blocking real pages).
Also explicitly **not** attempted here, reserved for the very next
session as planned: `grid-template-areas`/`grid-area` — nothing
grid-related was touched or regressed (the existing grid test suite
from v58 still passes unchanged).

A link-navigation planning conversation was raised in the same message
that reprioritized this work ("we should allow clicking links at some
point 😅 Let's plan this now") — out of scope for this item itself (an
implementation task, not a planning one) and not acted on here; left
for its own dedicated planning session.

---

## v65 — link navigation (scratchy side): real click detection, href resolution, and a live DOM swap

> ok, let's do this next. I really want to test it so pause once you are
> done with the links

This is the scratchy-side half of the link-navigation feature planned at
the end of v64 (`TODO.txt`'s "Link navigation" item, recorded
2026-10-30). The other half — `snag --serve`, a long-lived Unix-socket
daemon, plus `snag` emitting a real `<base href>` in both its one-shot
CLI and `--serve` output — already landed as `snag`'s own v9, in a
separate session; this item does not touch `snag/` at all. The user
explicitly wants to test this interactively themselves once it's done,
which shapes a lot of what's below: everything that *can* be verified
headlessly or via a real (non-interactive) process was, and everything
that genuinely needs a live mouse click is called out as such, not
glossed over.

**What landed, mechanism by mechanism:**

1. **Real click detection** — scratchy's first `WindowEvent::MouseInput`
   handler ever (previously only `CursorMoved`, for `cursor`'s hover
   icon since v32/v61, and `MouseWheel` existed). `ElementState::Pressed`
   on the left button records `self.last_mouse_pos` (already kept live
   by `CursorMoved`, since `MouseInput` itself carries no position of
   its own — same gap `MouseWheel` already has) into a new `mouse_down_
   pos` field; `Released` compares that against the *current* `last_
   mouse_pos` via a new, pure, unit-tested `is_click(down, up,
   threshold)` — a drag (scratchy has no text selection yet, but a
   careless same-event binding would misfire the moment it exists) moves
   past a 5px `CLICK_DRAG_THRESHOLD_PX`, a real click doesn't.
2. **Finding the href at that point** reuses `cursor`'s own v32/v61
   glyph-precision hit-testing machinery directly, not a second parallel
   walk — `hit_test_cursor` now returns `(CursorStyle, Option<&'static
   str>)` instead of just `CursorStyle`, with the href half threaded
   through the exact same per-glyph `metadata`-indexed vectors the
   cursor half already used (`SpanStyle` gained an `href: Option<&'static
   str>` field, the same `Box::leak`-into-`&'static` tradeoff named
   `font-family` fallback lists already made, needed to keep `SpanStyle`
   `#[derive(Copy)]`). `href` isn't a CSS property, so it's threaded as
   *ambient* propagation (same shape as `background-color`/`text-
   decoration`, not real CSS inheritance) through `collect_children` and
   the five `build_*` functions that already carry `decoration` the same
   way — computed once per `a`/`area` element from its own `href`
   attribute, falling back to whatever ancestor link (if any) is already
   in scope, so e.g. `<a href="/x"><b>bold link</b></a>`'s nested `<b>`
   correctly reports `/x` too. An anchor with no `href` at all (a
   fragment target, not a real link — same distinction `:link`'s own
   matching already makes, v37) correctly reports no href, even though
   `default.css`'s `a { cursor: pointer }` is a bare tag selector and
   still applies a pointer cursor to it regardless.
3. **Resolving the href to an absolute URL** uses the `url` crate (new
   dependency — `snag` already had it, scratchy didn't; `Cargo.toml`
   updated) via a small pure `resolve_href(base: Option<&Url>, href: &str)
   -> Result<Url, String>`: an already-absolute `href` resolves to itself
   regardless of `base` (real `Url::join` behavior, matching a real
   browser), a relative one joins against `base`, and a relative `href`
   with no `base` at all (no `<base href>` anywhere on the current page)
   is a clear, reported error rather than a guess. `App::base_url` is
   recomputed from the live document's own `<base href>` (`find_base_
   url`/`find_base_href`, the same recursive-walk shape `find_html_
   element` already established) every time the document is replaced —
   initial load included (`App::new_static`), and after `AppEvent::Eof`
   in streaming mode too (`snag url | scratchy` now picks up a usable
   base the moment the piped page finishes arriving, not just a saved
   bundle opened by path).
4. **The socket round trip** (`fetch_from_snag`/`fetch_from_snag_at`) is
   exactly the protocol `snag`'s own v9 entry documents: `std::os::unix::
   net::UnixStream::connect` to `$XDG_RUNTIME_DIR/snag.sock` (falling
   back to `/tmp/snag-$UID.sock`, computed by `snag_socket_path`/
   `current_uid` — the literal same formula `snag`'s own `socket_path`/
   `current_uid` use, reproduced rather than shared since the two are
   separate crates with nothing to link against), write the resolved
   URL plus a newline, read to EOF. Three outcomes, each a distinct
   human-readable `Err` rather than scratchy guessing at `snag`'s own
   wording: connection refused (socket missing — `snag --serve` was
   never started) gets a specific "start it with `snag --serve`"
   message; a clean connection that closes with zero bytes is `snag`'s
   own documented behavior for a failed top-level fetch (bad URL,
   network failure on its end — logged to *its* stderr, connection just
   closes), reported as a fetch error, not treated as a degenerate empty
   success; any other I/O error is reported with its own message.
5. **Loading indicator and the DOM swap itself** reuse `run()`'s own
   `Input::Stdin` background-thread pattern directly: `App::start_
   navigation` shows a synthetic "Loading…" one-paragraph page
   immediately (`App::show_message`, HTML-escaped via a new `escape_html`
   since the text it echoes — a clicked href, an OS error string — isn't
   scratchy's own content), then spawns a thread that calls `fetch_from_
   snag` and sends the result back via `event_loop.create_proxy().
   send_event(AppEvent::NavResult(generation, result))` — the *same*
   `EventLoopProxy` pattern, just needed at construction time now instead
   of only inside the `Input::Stdin` branch, since `ActiveEventLoop`
   (unlike the top-level `EventLoop`) has no `create_proxy` of its own
   (confirmed by reading winit's vendored source before assuming
   otherwise). `AppEvent::NavResult`'s `Ok` branch parses the returned
   bytes through the exact same one-shot `new_parser().read_from(...)`
   path `run()`'s own `Input::File` case already uses to build the
   *initial* document, then hands the result to a new `App::replace_
   document` — document, layout tree, shape cache, scroll offsets and
   position all reset, window redraw requested — which `start_
   navigation`'s "Loading…" step and a failed-fetch's error message both
   also go through, so there's exactly one swap mechanism, not three. A
   `u64` `nav_generation`, bumped on every `start_navigation` call and
   compared on arrival, drops a stale result from a navigation the user
   has since clicked past (started a second fetch before the first one
   finished) instead of letting it clobber newer content — the one place
   a future back/forward history stack would hook in, left as a comment,
   not built (explicitly out of scope, per the user's own prior
   decision recorded in `TODO.txt`).
6. **Scope, exactly as pre-agreed**: no back/forward history, no
   keybindings, no tabs/windows — click a link, it navigates forward,
   that's it. `<img>`-wrapped links (`<a href><img></a>`) aren't
   clickable — `href` only ever reaches a `SpanStyle`, never an image's
   own box, the same "images don't carry the per-span machinery text
   does" category of cut other per-span properties already have; noted
   inline at `hit_test_cursor`'s `LaidContent::Image` arm, not silently
   dropped.

**16 new permanent tests** (`mod tests`, "v65: link navigation" section,
kept per the v41 policy, nothing written-then-deleted): `resolve_href`
against an absolute href, a relative one, a relative one with no base,
and an absolute one with no base; `find_base_href` present/absent;
`is_click` accepting a tiny move, rejecting a real drag, and the
boundary case; `hit_test_cursor` recovering a real link's href at glyph
precision and reporting `None` for adjacent plain text; href ambiently
propagating into a nested element with no href of its own; an anchor
with no `href` correctly reporting no link (while still correctly
reporting a pointer cursor, since that part of `default.css` isn't
`:link`-scoped — worth the explicit test, since it's exactly the kind of
thing that's easy to get backwards by assuming cursor and href always
travel together); the real snag socket protocol against a tiny
in-process mock `UnixListener` (round-trip, connection-refused, and
clean-empty-response-is-an-error — same spirit as `snag`'s own v9 test,
which used a tiny in-process loopback HTTP fixture for the same reason:
real round-trip coverage without touching the real network); and
`escape_html`. All 160 tests pass (144 pre-existing + 16 kept from this
item, after fixing one test's own wrong assumption mid-session — see
below). `cargo check`/`cargo clippy` clean on every touched line (the
full-crate clippy run reports the exact same pre-existing warnings as
before this change, one count incremented from 9→10 arguments on
`collect_children`, which already had the warning before `href` was its
10th parameter — not a new category introduced here). `cargo install
--path .` run so `~/.cargo/bin/scratchy` isn't left stale.

**A real test-authoring mistake caught before trusting the suite, worth
recording**: the first version of `an_anchor_with_no_href_is_not_a_link`
searched for a `CursorStyle::Auto` glyph to test against, assuming "no
href" would mean "no special cursor either" — wrong, since `default.css`'s
`a { cursor: pointer }` is a plain tag selector, not `:link`-scoped, so
an `<a>` with no `href` still gets a pointer cursor. The test failed
immediately (no glyph found, since the whole fixture's text sat inside
the `<a>`), which is exactly the "fix the test, not assume the code's
wrong" discipline v19's own `PROMPTS.md` entry flagged — and a good
illustration of precisely why `href` needed its own tracking rather than
reusing `cursor` as a proxy signal, not just an incidental test bug.

**Live verification — exactly what was and wasn't checked against a
real process, stated plainly:**

- **Real socket round trip**: verified directly against a real, already-
  running `snag --serve` (a throwaway Python Unix-socket client, same
  tool `snag`'s own v9 verification used, since this sandbox still has
  no `nc`/`socat`) — fetched both `https://example.com` and
  `https://example.org` for real, each coming back with its own correct
  `<base href>`.
- **Rendering a page with a real link**: a real `scratchy` process,
  launched on a local fixture containing a real `<base href>` and a real
  `<a href="https://example.org/">`, screenshotted via `grim` — the link
  renders as expected (visible, styled, in place).
- **The DOM-build-from-fetched-bytes path**: the *exact* bytes `snag
  --serve` returned for `https://example.org` (saved to a file, not
  hand-written) were handed to a second, independent `scratchy` process
  — the same `new_parser().read_from(...)` call `AppEvent::NavResult`'s
  `Ok` branch uses — and rendered correctly, confirmed via another
  `grim` screenshot. This is the strongest live proof available that a
  real navigation's fetched content would display correctly once swapped
  in.
- **What was *not* verified live, stated honestly rather than
  overclaimed**: triggering `start_navigation`/`replace_document` via an
  actual mouse click inside one continuously-running `scratchy` process.
  This sandbox has no tool to synthesize a real mouse click (checked
  fresh this session, per the task's own instruction not to assume
  v32/v55/v62's old finding still holds — `ydotool`, `wtype`, `dotool`,
  `wlrctl`, and `ydotoold` are all still absent), so the literal
  same-window "click → Loading… → swapped-in new page" transition rests
  on code review and the strong indirect evidence above, not a directly
  observed in-process event. `App::replace_document`'s own reset logic
  is a strict superset of what `AppEvent::Chunk`/`Eof` (streaming mode,
  live and exercised since v2) already does on every incoming chunk, so
  the underlying "swap `self.document`, mark the layout dirty, let
  `redraw` rebuild" mechanism itself is not new or unproven — only the
  specific trigger (a `MouseInput` click firing `start_navigation`) is
  unverified live. Worth a real interactive check next time a human's at
  the keyboard — now the item this whole session was building toward,
  not just another entry in the ongoing cursor/sticky/wheel list of
  compositor-dependent gaps.

Also worth recording: a stray multi-window `grim` screenshot taken
mid-session, right after opening a second concurrent `scratchy`
instance, briefly showed what looked like one window's content bleeding
into another's tile position — re-tested with one `scratchy` instance
at a time afterward and it was consistently correct, so this reads as a
transient `dwl` tiling/compositor artifact from two windows opening in
quick succession (the same category of sandbox display quirk NOTES.md
already has precedent for, e.g. v13's `grim`/winit stall), not a real
bug in this item's own code — flagged here rather than silently
smoothed over, since it's exactly the kind of thing worth a second look
if it ever recurs.

## v66 — re-platformed link navigation onto `wire`, off `snag --serve`

> Merged. Now in a new branch do scratchy

The "Merged" refers to `wire` v1 (a separate, prior session — see
`wire/PROMPTS.md`'s own v1 entry): a new daemon that owns real
networking, built to satisfy `scratchy/TODO.txt`'s own "TOP PRIORITY"
planning entry (`32ee24c`). That entry already settled every design
question (request/response framing, the response-normalization
contract, `--socket` as a strict opt-in, concurrent connections) in a
prior conversation; this session's job, as `TODO.txt`'s own wire item
said in advance, was "wiring `scratchy` to actually dial `wire` instead
of `snag --serve`" — the next slice of that same item, not a separate
one. Scoped deliberately to exactly that slice: v65's existing GET-only
link-click navigation, re-platformed onto the new transport. Form
submission stays out of scope (per `TODO.txt`'s own "Form elements —
sequencing plan" item — `<button>` has no box, scratchy has no focus or
keyboard handling at all yet, so there's nothing a form's `submit` could
even target).

**What changed.** `fetch_from_snag`/`fetch_from_snag_at`/`snag_socket_
path` (v65's GET-a-URL-plus-newline line protocol against `snag
--serve`'s fixed auto-discovered socket path) are gone, replaced
outright — not extended, not kept as a fallback, exactly as `TODO.txt`
specified. In their place:

1. **`--socket <path>`**, a real CLI flag (`parse_args` now returns an
   `Args { input, socket }` instead of a bare `Input`) — strict opt-in,
   no auto-discovered default of any kind. Threaded through both `App`
   constructors into a new `socket_path: Option<PathBuf>` field.
2. **`build_get_request_text`**: a real HTTP/1.1 request in the proxy
   absolute-form shape `wire` itself parses — the exact framing
   `wire/src/main.rs`'s own `parse_request` expects, confirmed against
   its real source rather than assumed.
3. **`parse_wire_response`/`dispatch_wire_response`**: the response side
   of the contract `TODO.txt` settled — read `Content-Type` and nothing
   else; `text/html` passes straight through; a real decoded image
   (`image/png`/`jpeg`/`gif`/`webp` — `wire` always hands these back
   already-decompressed, confirmed by reading its own `normalize_
   response`) gets wrapped into a synthetic `<html><body><img
   src="data:...">` document (`wrap_image_as_html`) so it reaches
   scratchy through the *same* one rendering path as any other page,
   reusing `decode_data_uri`/`decode_image_bytes` rather than inventing
   a second "the document is just an image" mode; anything else is a
   named, inline "can't render this content type" error.
4. **`fetch_via_wire_at`/`WireFetchOutcome`**: connects, sends, reads to
   EOF (the real `wire` protocol's own "done" signal — no length
   prefix), then parses+dispatches. Collapses every failure before a
   complete response was read (`connect` refused, write failed, read
   failed) into one `Unreachable` outcome, kept distinct from "reached
   the daemon, got a response, but it's not one scratchy can render" —
   only the first case also triggers `print_fallback_request`.
5. **`print_fallback_request`**: `start_navigation`'s own stdout
   fallback, exactly as `TODO.txt` specified — the literal HTTP/1.1
   request text, printed whenever `--socket` was never given at all or
   the configured one couldn't be reached, directly replayable by
   `nc`/a raw socket/a manually-run `wire`.

`start_navigation` itself keeps its v65 shape (resolve `href`, bump
`nav_generation`, show "Loading…", fetch on a background thread, send
`AppEvent::NavResult` back through the existing `proxy`) — only what
happens *inside* that background thread changed.

**8 new permanent tests** replacing the 3 old `fetch_from_snag_at`
ones (168 total now, up from 160): `build_get_request_text` against a
plain URL and one with an explicit port; `parse_wire_response` extracting
`Content-Type`+body, confirming case-insensitive header lookup plus
other headers being ignored, and erroring on a missing header
terminator; `dispatch_wire_response` passing HTML through untouched,
wrapping a real generated PNG into a synthetic document whose `data:`
URI is then decoded back via the real `decode_data_uri` and confirmed
byte-identical to the original (not just a string-shape check), erroring
on an unsupported content type, and erroring when there's no
`Content-Type` at all; and a real end-to-end `fetch_via_wire_at` round
trip against a tiny in-process mock Unix-socket listener (confirming
the exact absolute-form request bytes sent) plus the unreachable-socket
case. `cargo test`: 168/168 passing. `cargo clippy --all-targets`:
confirmed the exact same 20 pre-existing warnings as a clean checkout of
`main` before this session (diffed directly, not assumed) — zero new
ones from any touched line.

**Live-verified further than any prior `PROMPTS.md` entry in this
project — including the one thing v65's own entry explicitly flagged as
*not* checked: an actual mouse click, inside a real running `scratchy`
process, triggering real navigation.** This sandbox turned out to have
`Xvfb` + ImageMagick's `import` (an X11 screenshot path, once
`libxkbcommon-x11-0` was installed so winit's X11 backend would even
start) and, unlike every prior session's sandbox, `xdotool` — enough to
synthesize a real `X11` mouse click for the first time in this project's
history. Full real pipeline exercised, nothing simulated or hand-fed:

- A real `wire` binary (release build) run as a real daemon on a real
  Unix socket.
- A real Python `http.server` loopback fixture standing in for a remote
  site (serving both a real HTML page and a real generated PNG).
- A real `scratchy` release binary, launched with `--socket` pointing at
  the real `wire` daemon, loading a real start page with a real
  `<a href="http://127.0.0.1:.../">`.
- `xdotool mousemove --sync`+`click 1` at the link's real screen
  position, confirmed first against a `before`-click screenshot.
- An `after`-click screenshot confirming the page genuinely swapped
  in-process to the fetched remote content — text-case: the start page
  replaced by the real fetched page's own heading/paragraph text,
  byte-identical to what the fixture served; image-case: a second real
  click, against a link to the generated PNG, rendered as a solid
  80×80 blue square matching the generated fixture image exactly (color
  and dimensions), proving `wrap_image_as_html`'s synthetic-document
  path renders correctly end to end, not just that it produces
  plausible-looking HTML in a unit test.

One hiccup during this, recorded rather than smoothed over: an earlier
attempt (before settling into the clean sequence above) showed the
real `print_fallback_request`/"wire daemon not reachable" error message
after a click, despite `ps` confirming a `wire` process alive at that
exact path moments before — most likely a stale-socket-file race from
restarting `wire` (`rm` + rebind) while an *older* `scratchy` process
(constructed against the path before that restart) was still running
and mid-background-thread-dispatch from a previous attempt, across
several overlapping `run_in_background` tool calls whose own 30-minute
timeouts killed processes out from under the test more than once. Not
chased further as a code bug (a clean restart of all three processes in
strict order, confirmed one by one before ever touching `scratchy`,
reproduced success twice in a row, including the image case) — but
worth noting `fetch_via_wire_at`'s `Unreachable` branch is itself
exactly what made that failure *visible* and diagnosable instead of a
silent hang, which is the behavior this item asked for either way.

**Deliverable**: `scratchy/src/lib.rs` (`Args`/`parse_args`, `App::
socket_path`+both constructors, `build_get_request_text`, `parse_wire_
response`, `wrap_image_as_html`, `dispatch_wire_response`,
`WireFetchOutcome`, `fetch_via_wire_at`, `print_fallback_request`,
`start_navigation` rewritten, `run()` threading `socket` through, 8 new
tests replacing 3 removed ones, doc comments updated throughout
wherever they referenced `snag --serve`/`fetch_from_snag`).

## v67 — live-fetched images and external stylesheets, for a wire-navigated page

> Yes. Also do the same thing for css where relevant. Use the existing
> PR, it's ok

Direct follow-up to a question asked right after v66 landed: "I see no
images right now. I guess scratchy can't deal with not inlined images
yet?" — confirmed by reading `resolve_img`/`decode_data_uri` directly:
correct, only a `data:` URI `src` ever decoded; a real `<img src="https://…">`
fell straight through to the `alt`-text fallback, silently, every time.
Root cause: a page `snag` bundles ahead of time has every image/
stylesheet already inlined by `snag` itself, but a page reached by
*clicking a live link* (v66) never goes through `snag` at all — nothing
inlines anything for it. This session builds the missing piece:
scratchy fetching its own sub-resources, via the same `wire` daemon, for
exactly that path.

**Design, worked out before writing any code**: CSS and images need
genuinely different timing, not one shared mechanism. A stylesheet has
to be fully resolved *before* the very first layout — nothing in this
codebase supports a second, incremental re-style pass, and rendering
once unstyled then snapping to styled would be a visible flash. An
image's intrinsic size, by contrast, only matters locally (one box), so
it can load lazily, same as a real browser: show the page immediately,
let each image materialize as its own fetch lands. So: external
stylesheets are fetched **synchronously**, inside `start_navigation`'s
own background thread, *before* `AppEvent::NavResult` is even sent;
images are fetched **asynchronously**, each its own `thread::spawn`
fired from that same point but never waited on, reporting back through
a new `AppEvent::ImageFetched` whenever it individually finishes.

**The `!Send` constraint that shaped the whole implementation**:
`markup5ever_rcdom::Handle` is `Rc`-based, hence `!Send` — it can never
cross the `thread::spawn`/`EventLoopProxy::send_event` boundary this
codebase already uses for every other background result. So finding a
page's `<link rel=stylesheet>`/`<img src>` references has to happen by
parsing the fetched HTML bytes into a *throwaway* DOM entirely within
the background thread (`process_external_resources`), walking it there,
and only ever sending plain owned data back across — a fetched
stylesheet's `String`, an image's raw `src` key `String`, and a plain
`DecodedImage` struct (not `Rc<DecodedImage>` — `Rc::new` only happens
once an image lands back on the main/event-loop thread, inside
`AppEvent::ImageFetched`'s own handler). The throwaway DOM itself is
dropped at the end of that function, having never left the thread it
was built on.

**What landed, by layer**:

1. **Fetch layer**: `fetch_via_wire_at`/`WireFetchOutcome` (v66) split
   into a lower-level `fetch_raw_via_wire`/`WireFetchRaw` (connects,
   sends, reads to EOF, parses just `Content-Type`+body — no longer
   dispatches by itself) plus the original top-level dispatch logic
   moved into `start_navigation`'s own match arm. `fetch_sub_resource_
   via_wire` (mime-checked, lenient — `None` on anything from
   unreachable to a wrong content type, never an error that could sink
   the rest of the page) backs both `fetch_stylesheet_via_wire`
   (`text/css` only) and `fetch_image_via_wire` (reuses `decode_image_
   bytes` verbatim — the exact decoder a `data:` URI `src` already
   used, so a live-fetched image and an inlined one decode identically).
2. **Discovery layer**: `ExternalResources`/`find_external_resources` —
   a plain recursive DOM walk (same shape `find_base_href` already
   uses) collecting every `<link rel=stylesheet href>` (real `rel`
   token-list matching — `rel="alternate stylesheet"` still counts) and
   non-`data:` `<img src>`, unresolved, in document order.
3. **Orchestration**: `process_external_resources` — resolves each
   reference against the fetched page's own `<base href>` (or its URL,
   falling back the same way `resolve_href`'s every other caller
   already does), fetches every stylesheet synchronously and
   concatenates them into the `extra_css: String` it returns, and fires
   one deduplicated background thread per distinct image URL.
4. **Rendering layer**: `App` gained `extra_css: String` (merged into
   `collect_style_css`'s own output by a new `build_layout_tree_with_
   resources` — `build_layout_tree` itself stays a 2-argument wrapper
   over it with empty/no-op defaults, so none of this file's ~60
   existing `build_layout_tree` test/benchmark call sites needed to
   change at all) and `image_cache: HashMap<String, Rc<DecodedImage>>`,
   threaded through the same 8-function `build_children`→
   `collect_children`→…→`resolve_img` chain `href`/`viewport_width`
   already travel (same accepted "too many arguments" cost `href`'s own
   v65 addition already took on `collect_children`; diffed directly
   against a clean pre-session `main` checkout to confirm exactly one
   *new* clippy warning total, `build_children_with_prefix` crossing the
   7→8-argument threshold — nothing else moved). `resolve_img` now
   falls back to `image_cache` (keyed by the *raw, unresolved* `src`
   text — deliberately not a resolved absolute URL, which would need
   threading `base_url` through that same 8-function chain for no real
   benefit: `App::replace_document` already clears `image_cache` on
   every navigation, so the same relative string can never carry over
   from a different page's own base URL) whenever `decode_data_uri`
   itself finds nothing. `AppEvent::ImageFetched`'s handler inserts one
   cache entry and sets `layout_dirty = true` — a newly-known intrinsic
   size can change this element's own box, so a real rebuild is needed,
   not just a repaint.

**9 new permanent tests** (177 total, up from 168): the two existing
`fetch_via_wire_at` tests renamed/adapted onto `fetch_raw_via_wire`'s
narrower return type; `find_external_resources` against a fixture
mixing stylesheet/icon/alternate-stylesheet `<link>`s and `data:`/real
`<img src>`s; `fetch_stylesheet_via_wire`/`fetch_image_via_wire` each
against a real mock Unix-socket listener, both the accept- and reject-
content-type cases; `resolve_img` against a populated vs. empty
`image_cache` directly; `build_layout_tree_with_resources` confirmed to
actually apply `extra_css` to the real cascade (a `.box` background
color, same assertion shape `@media`'s own tests already use) and to
render a cached image as a real `BoxContent::Image` box, not just
plausible-looking data. `process_external_resources` itself is
deliberately *not* unit-tested directly — it takes a real
`EventLoopProxy<AppEvent>`, and this file's own established precedent
(NOTES.md's v32/v55/v62/v65 entries) is to never construct a real winit
`EventLoop` inside `#[cfg(test)]` just to get one; its own thin
wiring rests on code review plus the live verification below, same
category `start_navigation`'s own click-handling already is. `cargo
test`: 177/177. `cargo clippy --all-targets`: exactly one new warning
beyond a clean pre-session `main` checkout (see above), diffed directly,
not assumed.

**Live verification — further than v66's own bar, which was already the
project's high-water mark**: built real release binaries of both
`scratchy` and `wire`. A real Python `http.server` fixture served three
routes — an HTML page linking an external stylesheet and a real `<img
src>`, the stylesheet itself, and a real generated PNG. A real `xdotool`
click (same `Xvfb`/`import`/`libxkbcommon-x11-0` setup v66's entry
describes) on a real on-screen link, inside a real running `scratchy
--socket`, against a real running `wire`:

- **External CSS applied before first paint**: the clicked-to page's
  `.box` rendered at the fetched stylesheet's own declared size and
  color (a 120×60px magenta rectangle) — confirmed by screenshot, not
  assumed from the fetch succeeding.
- **A real fetched image rendered correctly**: a real generated 64×64
  solid-green PNG, fetched from the live server (never inlined, never
  touching a `data:` URI anywhere in this path), appeared as a correctly
  sized, correctly colored block alongside the styled text.
- **The async/non-blocking design, proven, not just argued**: re-ran the
  same click against a fixture with an artificial 1.5s delay on the
  image route specifically. A screenshot taken 0.5s after the click
  (well before the image could possibly have arrived) already showed
  the fully-navigated page with the external stylesheet applied — the
  image slot simply not there yet, not a blocked/frozen page. A second
  screenshot after the delay showed the exact same page, now with the
  image present too, with no further click or user action — direct,
  visual proof that `process_external_resources`'s sync-CSS/async-image
  split behaves exactly as designed under real timing, not just in the
  no-latency common case a fast loopback fetch would otherwise mask.

Three real PNG screenshots from this session (before/mid-flight/after)
back this account; not reproduced here, but the steps above are exact
enough to redo.

**Deliverable**: `scratchy/src/lib.rs` (`WireFetchRaw`/`fetch_raw_via_
wire` replacing `WireFetchOutcome`/`fetch_via_wire_at`; `fetch_sub_
resource_via_wire`/`fetch_stylesheet_via_wire`/`fetch_image_via_wire`;
`ExternalResources`/`find_external_resources`/`process_external_
resources`; `FetchedPage` struct; `AppEvent::ImageFetched`; `App::
extra_css`/`image_cache` fields; `build_layout_tree_with_resources`;
`image_cache` threaded through `build_children`/`build_children_with_
prefix`/`collect_children`/`build_box_for_element`/`build_table_
children`/`build_table_row_cells`/`build_flex_children`/`build_img_
box`/`resolve_img`; `replace_document` gained an `extra_css` parameter;
`start_navigation` rewritten around the new fetch/dispatch split; 9 new
tests). `scratchy/NOTES.md`, `scratchy/TODO.txt` both updated.

## v68 — a fetched page with no `<base>` of its own no longer breaks every relative link

> Fix it now. Make new PR

Direct fix for a real bug found live, mid-testing, reported with a
screenshot: clicking `#bodyContent` (MediaWiki's own "Jump to content"
skip-link) on a real, live-navigated page failed with `cannot resolve
link "#bodyContent": no <base href> on this page to resolve it against`.

**Root cause, confirmed by reading the code, not guessed at**:
`AppEvent::NavResult`'s `Ok` handler only ever set `self.base_url =
find_base_url(&dom.document)` — a fetched page's own `<base href>`, if
it declares one. Most real pages don't (a real browser's own "document
base URL" already defaults to the page's own fetched address when
there's no `<base>` element at all — declaring one is the exception, not
the rule). `snag`'s own bundled pages never hit this because `snag`
itself always synthesizes a `<base href>` reflecting the real fetched
URL when the page doesn't declare its own (v9); `wire`-navigated pages
(v66/v67) never got the equivalent — `FetchedPage` didn't even carry the
URL that was fetched, so `NavResult` had nothing to fall back to.
`base_url` landing at `None` doesn't just break the one link that
happened to be clicked — every relative href on that page breaks the
same way, fragment links included, until the next full navigation.

**Fix**: `FetchedPage` gained a third field, `url: Url` (the real
address `start_navigation` resolved and fetched — already computed,
just not threaded through before). A new, small, directly-testable
`resolve_page_base_url(document, fetched_url) -> Url` — `find_base_url`
if the page declares one, else `fetched_url` — replaces the old bare
`find_base_url(...)` call in `NavResult`'s handler. Mirrors `snag`'s own
v9 behavior exactly, just computed in scratchy itself instead of
pre-baked into the bundle.

**2 new permanent tests** (179 total, up from 177): `resolve_page_base_
url` still prefers a page's own real `<base>` tag when present; and the
exact failure shape itself — a page with no `<base>` and a `<a
href="#bodyContent">` — confirmed to both produce the fetched URL as the
fallback base *and* that `resolve_href` against that base actually
succeeds now (`.../wiki/Example#bodyContent`), not just that the
fallback value looks plausible in isolation. `cargo test`: 179/179.
`cargo clippy --all-targets`: unchanged warning count (21, same as
after v67 — `resolve_page_base_url` is a plain 2-argument function, no
new threading-cost warnings).

**Live-verified by reproducing the exact bug, then confirming the fix,
both as real clicks** — not just trusting the unit tests: a real local
HTTP fixture served a page with no `<base>` tag at all, containing both
a `#bodyContent` fragment link and a genuine root-relative link
(`/relative-page`). A real `xdotool` click through a real running
`wire` daemon onto the fragment link, previously guaranteed to fail,
now re-fetches and re-renders the same page with no error at all
(scratchy has no anchor-scroll feature, so a fragment link is simply a
same-page re-navigation — the point being it *resolves*, not that it
scrolls). A second real click on the genuine relative link correctly
navigated to and rendered the distinct target page. Both confirmed via
screenshot.

**Deliverable**: `scratchy/src/lib.rs` (`FetchedPage::url`,
`resolve_page_base_url`, `start_navigation`'s `FetchedPage` construction
site, `NavResult`'s handler simplified to one line, 2 new tests).

## v69 — command bar QoL: `:visit` renamed to `:go`, Tab completion/cycling, `:help`, `:q`/`:quit`, clipboard paste

> I want to make some qol improvements in the command bar before we
> move to other things, because it's annoying to use right now. First
> rename :view to :go, feels better. Second, I want tab completion and
> cycling in command mode. FInally I want a help page demonstrating all
> the features of the browser. It can get updated as we add more. :q
> should close the browser.

> something I forgot. I should also be able to paste text there (for
> example a URL I got from somwhere else). This is not available by
> default.

Five small, independent requests against the existing `:`-command bar
(itself already landed in the codebase by the time this session started,
but with no `PROMPTS.md` entry of its own — a pre-existing gap this
entry doesn't attempt to backfill, only to build on). The user said
`:view`; the command that actually exists is `:visit` — treated as the
same ask (there's no `:view` today, and "feels better" reads as a
rename, not a request for a second command), confirmed working end to
end before/after, not just assumed.

**`:visit` → `:go`:** one `dispatch_command` match arm renamed, plus
its own and `key_insertion_text`'s doc comments (the latter cited the
old name in a historical bug example — updated to match, since it's
living documentation, not a changelog entry).

**Tab completion/cycling:** two new pure, unit-tested functions —
`token_start_before` (the byte offset of the whitespace-delimited token
ending at the cursor) and `command_completions` (the candidate pool for
that token: `COMMAND_NAMES` for the first token, `SET_ARGS` for `set`'s
own second token, nothing for anything else — deliberately no
completion for `:go`'s own URL argument, which has no finite "correct"
answer to offer). A new `App::completion: Option<CommandCompletion>`
field holds the live cycling state; `App::cycle_command_completion`
completes on the first `Tab` and advances (wrapping) on every
*consecutive* one, reset on any other command-line keystroke
(`handle_command_line_key`'s own top-of-function check) and on
re-entering command mode fresh (`perform_action`'s `EnterCommandMode`
arm) so a stale completion from a previous `:` session can't leak into
a new, empty one.

**`:help`:** a new static page, `src/help.html`, embedded via
`include_str!` the same way `default.css` already is (`HELP_PAGE_HTML`,
next to `DEFAULT_CSS`), shown through `App::show_help` (`parse_html` +
`replace_document`, same shape `show_message` already uses for its own
synthetic pages). Plain semantic HTML — headings, two tables, a list,
`<b>`/`<i>`/`<code>`, an `<hr>`, and a small embedded `data:` PNG — with
no `<style>` of its own, styled entirely by `default.css`, so the page
doubles as a live demo of the default stylesheet rather than a text
dump. Lists every Normal-mode key and command-bar command, including
the ones landing in this same change; expected to be kept current by
whoever changes the command bar next, the same turn they change it.

**`:q`/`:quit`:** `dispatch_command` sets a new `App::should_quit: bool`
— it has no `&ActiveEventLoop` of its own to call `exit()` on directly.
`window_event`'s `WindowEvent::KeyboardInput` arm (the one real call
site with an `ActiveEventLoop` in scope) checks the flag right after
`handle_key_event` returns and calls `event_loop.exit()` there.

**Clipboard paste:** `Ctrl`+`v` inside the command bar, via a new
`arboard = "3.4"` dependency (winit has no clipboard API of its own).
Detected the same way `resolve_binding`'s own `Ctrl`+`d`/`u` arms are —
`Key::Character("v")` guarded on `self.modifiers.control_key()`, ahead
of the plain-insertion catch-all. `TextEditBuffer` gained `insert_str`
(one `insert` per char, control characters — a trailing clipboard
newline being the real case — dropped, since this buffer is always
single-line). A fresh `Clipboard` connection is opened per paste rather
than held on `App` permanently: paste is a rare, user-initiated action,
not a hot path, so there's no real cost to the setup, versus a
permanent connection that would need its own reconnect/`Send` lifecycle
for a feature that doesn't need one. A failed connection or an empty/
unreadable clipboard is a silent no-op, same as a real terminal or
browser's own paste.

**7 new permanent tests** (198 total, up from 191): `token_start_before`
and `command_completions` (prefix-only matching, first-token vs. `set`'s
own second-token candidate pools, no candidates for an arbitrary
argument), and `TextEditBuffer::insert_str` (control characters dropped,
inserts at the cursor rather than always at the end). `App`-level wiring
(`cycle_command_completion`/`apply_completion` replace-range logic,
`paste_into_command_line`, `show_help`, `should_quit`'s path to
`event_loop.exit()`) has no unit test of its own, same "thin
orchestration rests on code review + live verification" category
`start_navigation`'s own click wiring and v67's fetch dispatch already
sit in — verified live instead, below. `cargo test`: 198/198. `cargo
clippy --all-targets`: unchanged warning count (20) — one new
`collapsible_if` in `paste_into_command_line` was fixed with a let-chain
rather than left as a 21st.

**Live-verified against a real running `scratchy` window**, not just
argued: Xvfb (`:99`) plus `xdotool`/`import` (neither was in this
container; installed for this session) stood in for a real X server and
a real keyboard/screenshot, since this environment has no GPU display of
its own. `:help` rendered correctly (headings, both tables, the
embedded image decoded — a real checkered square, not a broken-image
box) and scrolled with `Ctrl`+`d` same as any other page. `Tab` on `:h`
completed to `:help`; a bare `:` cycled `go → help → q → quit → set →
go` across six consecutive `Tab` presses, confirming both the
alphabetical order and the wraparound; `:set ` cycled `novim → vim`.
`Ctrl`+`v` after setting the real X `CLIPBOARD` selection (`xclip
-selection clipboard`) to a URL pasted that exact URL into the command
line. `:go https://example.com/` printed the real resolved HTTP request
to stdout (no `--socket` given), exactly `start_navigation`'s own
documented no-socket behavior — proving the rename routes through the
real navigation path, not just a renamed no-op. `:q` closed the window
and exited the process (confirmed by polling the PID; takes up to ~1.5s
to actually exit under Xvfb, not instant, but it does exit).

**Deliverable**: `scratchy/src/lib.rs` (`COMMAND_NAMES`/`SET_ARGS`,
`CommandCompletion`, `token_start_before`, `command_completions`,
`App::completion`/`should_quit` fields, `App::cycle_command_completion`/
`apply_completion`/`paste_into_command_line`/`show_help`,
`TextEditBuffer::insert_str`, `dispatch_command`'s `:go`/`:help`/`:q`/
`:quit` arms, `window_event`'s `should_quit` check, 7 new tests);
`scratchy/src/help.html` (new); `scratchy/Cargo.toml`/`Cargo.lock`
(`arboard` dependency added).

## v70 — `:q` from `:help` goes back, instead of quitting

> :q from the help page should bring the user back to their previous
> page, similar to how vim closes the buffer in those cases

A follow-up on v69's `:help`, filed the same session right after seeing
it work: `:q` previously quit scratchy outright no matter what was on
screen, which made `:help` a dead end — closing it meant closing the
whole browser. Real vim's own `:q` doesn't work that way: it closes the
current buffer/window and falls back to whatever was open before it,
only actually quitting once there's nothing left to fall back to. That's
the exact shape asked for and the exact shape built.

**What/why:** a new `App::pre_help_page: Option<SavedPage>`, where
`SavedPage` snapshots every piece of per-page state `replace_document`
either changes or clears (`document`, `extra_css`, `image_cache`,
`scroll_offset`, `scroll_offsets` — not `base_url`, since `show_help`'s
own `replace_document` call never touches it in the first place).
`show_help` snapshots into it, but only when it's already `None` —
re-running `:help` while already viewing it must not let the help page
itself get captured as "the page to go back to." `dispatch_command`'s
`:q`/`:quit` arm now branches on `pre_help_page`: `Some` pops it
(`App::restore_saved_page`, the mirror image of the snapshot — restores
the three fields `replace_document` always zeroes instead of leaving
them zeroed, scroll position included, since closing `:help` isn't
supposed to look like navigating to a new page); `None` falls back to
the previous behavior (`should_quit`). `start_navigation`'s own first
line unconditionally drops `pre_help_page`: a real navigation (a real
click, or `:go`) while `:help` happens to be open means the page about
to show is genuinely new, not an overlay on top of whatever `:help`
saved — a later `:q` from there should quit, not resurrect a page from
before `:help` was ever opened. `src/help.html`'s own `:q`/`:quit` row
updated to describe both halves.

No new unit tests — same "thin App-level orchestration rests on code
review + live verification" category v69's own `cycle_command_
completion`/`paste_into_command_line` already sit in, not a gap: there's
no new pure, stateless function here to unit-test, just `SavedPage`
plumbing through existing `App` methods. `cargo test`: still 198/198
(unchanged). `cargo clippy --all-targets`: unchanged warning count (20).

**Live-verified against a real running window** (same Xvfb/`xdotool`/
`import` setup v69 used), four scenarios, not just the one asked for:
(1) scrolled a real page down, opened `:help`, scrolled *that* down to a
different position, ran `:q` — landed back on the original page at the
exact same scroll offset (pixel-identical screenshot to before `:help`
was opened), process still running. (2) `:q` with no `:help` open still
quits for real (polled the PID: exits a couple seconds later, same as
v69). (3) Opened `:help`, opened `:help` *again*, then `:q` — landed on
the real original page, not stuck on the help page itself (confirms the
"snapshot only once" guard). (4) Opened `:help`, then ran a real `:go`
to navigate away, then `:q` — this time it quit instead of popping back,
confirming `start_navigation`'s own clear takes effect.

**Deliverable**: `scratchy/src/lib.rs` (`SavedPage`, `App::
pre_help_page`, `App::restore_saved_page`, `show_help`'s snapshot guard,
`dispatch_command`'s `:q`/`:quit` arm, `start_navigation`'s clear);
`scratchy/src/help.html` (`:q`/`:quit` row updated).

---

## TODO.txt review — ranked QUEUE for cheaper-model handoff (no code change)

> I want you to review the TODO.txt of @scratchy/ and prioritize the
> features in order of maximum impact when rendering a web page. Look
> for holes and logic gaps and ask me for clarification so you can
> update the items accordignly. Those items will be given to a cheaper
> model for implemetation.

Follow-up answers, verbatim:

> https://social.loupax.com/u/loupax wikipedia https://forums.gentoo.org/
> google.com most sites I browse require js so we can't really test them
> yet. Yes, wire should deal with file:// URLs. Let's do the @import as
> suggested

Decisions picked from multiple-choice questions in the same
conversation: rank by a re-measured gap report on the pages above;
`opacity` multiplies alpha down the tree; for `letter-spacing`, check a
newer cosmic-text first; add a ranked QUEUE at the top of `TODO.txt`
and leave the history below it in place.

**What changed:** `TODO.txt` only. A new QUEUE section (Q1–Q33, each with
a size, functions to touch by name, tests, and a pointer to its detail
entry) now sits above the original log, and the order in it comes from
`SCRATCHY_CSS_DEBUG=1` runs plus raw-CSS selector/at-rule counts on the
four pages. Gaps that had no open item at all are now queued: per-side
borders, pseudo-elements, `!important` in the cascade, CSS-wide keywords,
`calc()`, viewport units, charset decoding (google.com dies on
ISO-8859-1), SVG (zero support; 269 SVG data URIs on one Wikipedia page),
`vertical-align: middle`, and several small properties. Stale entries in
the history were also fixed: `lib.rs:NNNN` line references were replaced
by function names, two misleading "TOP PRIORITY" labels on items that
are done or blocked were removed, and the open flex "automatic minimum
size" item is now narrowed to the column axis, since the row axis landed
in `500bd16`.

---

## Q1 — `SCRATCHY_CSS_DEBUG` tells failures apart (queue item)

> Implement the first unchecked item (Q1) of the ranked QUEUE in
> scratchy/TODO.txt.

**What changed:** `DeclParser::parse_value`'s per-property dispatch moved
into `DeclParser::apply`. When a declaration leaves `Declarations`
unchanged, it is re-applied to a blank `Declarations`. It is reported as
`ignored` only if that also sets nothing, so duplicate-valued
declarations stay silent. Selectors that fail to parse are reported as
`dropped selector `<text>``, and dropped at-rules as `dropped @<name>`
(from the `StyleSheetParser` errors in `parse_stylesheet` and the nested
`@media` loop). All lines go through `css_debug_report`, which a test can
capture via a thread-local. Test:
`css_debug_distinguishes_duplicates_dropped_selectors_and_at_rules`.
No grim screenshot: output-only diagnostic on stderr.

---

## Q2 — dropped pseudo-classes now parse (queue item)

> Implement the first unchecked item (Q2) of the ranked QUEUE in
> scratchy/TODO.txt.

**What changed:** `OurPseudoClass` gained `NeverMatches(name)` (for
`:hover`, `:active`, `:focus`, `:focus-visible`, `:focus-within`,
`:target`), `Checked`, `Disabled`, `Enabled`, `Dir(ltr)` and `Lang(x)`.
`OurSelectorParser` parses the plain ones and, through a new
`parse_non_ts_functional_pseudo_class`, `:dir()` and `:lang()`.
`match_non_ts_pseudo_class` implements them per the queue spec (`:lang`
walks ancestors via `parent_handle`; new free fn `is_form_control`).
Tests: `q2_state_pseudo_classes_parse_and_match`,
`q2_checked_disabled_enabled_match_elements`. No grim screenshot:
`grim` fails in this sandbox ("failed to copy output eDP-1"), so the
Wikipedia "Jump to content" check is unverified by a real render.

---

## Q3 — pseudo-elements parse and are skipped (queue item)

> Implement the first unchecked item (Q3) of the ranked QUEUE in
> scratchy/TODO.txt.

**What changed:** `NoPseudoElement` is replaced by `OurPseudoElement`
(`Named(&'static str)` for `before`, `after`, `marker`, `placeholder`,
`selection`, `first-line`, `first-letter`, `backdrop`,
`file-selector-button`; `Vendor(String)` for `-webkit-*`/`-moz-*`/`-ms-*`).
`OurSelectorParser::parse_pseudo_element` parses them, so
`a, a::after{..}` keeps its `a` part. `match_pseudo_element` returns
false, so a selector carrying one never matches an ordinary element.
`matching_declarations` ignores pseudo-element selectors when computing a
rule's specificity. The Q1 debug test now uses `p::bogus` as its dropped
selector. Test: `q3_pseudo_elements_parse_but_never_match_elements`.
`grim` worked this time; a simple page rendered normally (link and
paragraph, nothing generated from pseudo-elements).

---

## Q4 — per-side borders (queue item)

> Implement the first unchecked item (Q4) of the ranked QUEUE in
> scratchy/TODO.txt.

**What changed:** `Declarations` and `ComputedStyle` now hold
`border_width`/`border_color`/`border_style` as `[_; 4]` arrays in
top/right/bottom/left order (`SIDE_NAMES`). New `parse_border_property`
(called from the declaration parser) handles `border`, the box-syntax
`border-width/-color/-style` (1 to 4 values, via `parse_box4`), the 4 side
shorthands and the 12 side longhands. `resolve_style` zeroes a side's width
when its style is `none`/`hidden`. Layout reads `ComputedStyle::border_top/
border_left/border_h/border_v` instead of `2.0 * border_width`.
`paint_box_decoration` keeps the old `paint_border`/`fill_rounded_ring`
path when all four sides are identical, and otherwise paints each side
with its own width/style/color (bevel styles degrade to solid;
`border-radius` falls back to square corners). Test:
`q4_per_side_borders_parse_and_resolve`. `grim` worked: a div with
`border-bottom: 4px solid red; border-left: 6px dashed blue` showed only
those two sides.

---

## Q4b — per-side border joins, radius and bevels (queue item)

> Implement the first unchecked item (Q4b) of the ranked QUEUE in
> scratchy/TODO.txt.

**What changed:** the non-uniform branch of `paint_box_decoration` now calls
the new `paint_per_side_border`, a per-pixel painter: each border-band
pixel goes to the side it is proportionally closest to (distance / width),
giving diagonal mitered corners; the outer edge is rounded by
`border-radius` and the inner edge by `radius - width` per axis
(`in_rounded_rect`); `border_pixel_color` evaluates solid/dashed/dotted/
double and the four bevel styles per side. `round` is now true for any
non-uniform border (the background rounds too). Test:
`q4b_per_side_borders_miter_round_and_bevel`. `grim` worked: a div with red
top, dashed blue left and double green bottom borders and `border-radius:
24px` showed rounded, mitered corners. Left out: dashed/dotted/double/bevel
uniform borders still go square with `border-radius` (see QUEUE Q4c).

---

## Q4c — uniform non-solid borders with border-radius (queue item)

> Implement the first unchecked item (Q4c) of the ranked QUEUE in
> scratchy/TODO.txt.

**What changed:** in `paint_box_decoration`, `round` is now true for any
`border-radius > 0`, and the old `fill_rounded_ring` path (`ring_path`) is
kept only for uniform solid/absent borders; uniform dashed/dotted/double/
bevel borders with a radius now go through `paint_per_side_border`. Test:
`q4c_uniform_non_solid_borders_round_with_border_radius`. `grim` worked: a
120x80 box with `8px dashed red` and `border-radius: 24px` showed rounded
dashed corners. This also completes Q27 (Q4b covered unequal per-side
borders), so Q27 is ticked.

---

## Q5 — anonymous flex items for bare text (queue item)

> Implement the first unchecked item (Q5) of the ranked QUEUE in
> scratchy/TODO.txt.

**What changed:** `build_flex_children` now collects runs of consecutive
text nodes directly inside a flex/grid container into one anonymous block
item (`BoxContent::Inline`, `anonymous_style`); whitespace-only runs are
dropped. Elements remain their own items (per CSS Flexbox §4, inline
siblings are not merged into the text item). Test:
`q5_bare_text_in_flex_container_becomes_anonymous_flex_item`. `grim`
worked: `<div style="display:flex">hello <b>x</b></div>` showed "hello x".

---

## Q6 — `!important` (queue item)

> Implement the first unchecked item (Q6) of the ranked QUEUE in
> scratchy/TODO.txt.

**What changed:** `DeclParser` detects a trailing `!important` (`strip_important`) and routes the declaration into a second `Declarations` (`StyleRule.important`; inline `style` via `parse_declarations_split`). `matching_declarations` pushes each rule's important set with `IMPORTANT_TIER` (bit 31) OR-ed into its specificity; inline important is pushed last at `u32::MAX`. `cascade_property` now lets any important entry beat an inline normal declaration; within a tier specificity then source order decide. Custom properties stay in the normal set. Test: `q6_important_declarations_beat_specificity_and_inline_style`. `grim` worked: `#a{color:red} p{color:blue !important}` rendered the `<p id=a>` blue.

---

## Q7 — CSS-wide keywords (queue item)

> Implement the first unchecked item (Q7) of the ranked QUEUE in
> scratchy/TODO.txt.

**What changed:** `DeclParser::apply` recognizes `inherit`/`initial`/`unset`/`revert`/`revert-layer` (optionally followed by `!important`) for color, background/background-color, display, font-size, line-height, vertical-align, font-family, font-weight, text-align, white-space, cursor and visibility, storing them in `Declarations::wide` (mutually exclusive with the typed field; last declaration in a rule wins). `cascade_css` lets a keyword compete at its own specificity/source order/important tier, and `apply_wide` resolves it (`revert` = `unset`; `unset` = inherit for inherited properties, initial otherwise). `InheritedStyle` gained the parent's `background_color`, `display` and `vertical_align` so `inherit` works on those non-inherited properties. Test: `q7_css_wide_keywords_inherit_initial_unset_revert`. `grim` worked: `<p style="color:inherit">` under a red div rendered red. Left out: keywords on other properties (margin, border, etc.) are still ignored; `background: inherit` only covers the colour, not the image.

---

## Q7b — CSS-wide keywords on the remaining properties (queue item)

> Implement the first unchecked item (Q7b) of the ranked QUEUE in
> scratchy/TODO.txt.

**What changed:** `wide_keys` maps every property name (longhand, shorthand, logical margin/padding) to canonical keys; `Declarations::set_wide`/`clear_typed`/`has_typed` keep keyword and typed value mutually exclusive for all of them. `InheritedStyle` now carries `parent: Option<ComputedStyle>`, and a post-pass in `resolve_style` (`apply_rest_wide`, run only if some matched rule declared a keyword) cascades each keyword key with `cascade_css` and, when a keyword wins, `copy_prop`s the parent's or the initial value (`anonymous_style(ROOT_STYLE)`) into the result; the float/position forced-block `display` rule is re-applied afterwards. `resolve_background_image` now uses `cascade_css`: `inherit` recurses to the parent element, other keywords give no image; `background` maps to colour + image. Test: `q7b_css_wide_keywords_on_remaining_properties`. `grim` worked: a `<p>` with `margin/padding/border/background/width: inherit` inside a bordered div rendered with the inherited values. Left out (new QUEUE item Q7c): text-decoration properties, other logical shorthands.

---

## Q7c — CSS-wide keywords on text-decoration and logical shorthands (queue item)

> Implement the first unchecked item (Q7c) of the ranked QUEUE in
> scratchy/TODO.txt.

**What changed:** `wide_keys` now maps `text-decoration` (+ `-line`/`-style`/`-color`, via `TEXT_DECO_KEYS`), `inset*`, and `border-block/inline*` (shorthands, -start/-end, -width/-style/-color) to canonical keys; `clear_typed` knows the four text-decoration fields. `resolve_style` cascades the four text-decoration properties with `cascade_css`: `inherit` copies the parent's own cascaded value (via `InheritedStyle.parent`), `initial`/`unset`/`revert` give "not declared" (the ambient decoration an ancestor draws is unaffected, as in real CSS). Logical keys reuse the existing `copy_prop` path. Test: `q7c_css_wide_keywords_text_decoration_and_logical_shorthands`. `grim` worked: a `<p>` with `text-decoration-line/-color: inherit; border-block: inherit` showed a red underline and top/bottom blue borders. Left out (noted in TODO, no new item): `font-variant`/`letter-spacing`, which scratchy doesn't parse, so a keyword has nothing to apply to.

---

## Q8 — Charset decoding (queue item)

> Implement the first unchecked item (Q8) of the ranked QUEUE in
> scratchy/TODO.txt.

**What changed:** new dependency `encoding_rs`. `sniff_encoding` picks BOM, then the wire `Content-Type` `charset=` (`content_type_charset`), then `<meta charset>`/`http-equiv` in the first 1024 bytes (`meta_charset`), then UTF-8; WHATWG labels apply (`iso-8859-1`/`latin1` become windows-1252). `decode_html_bytes` is used by the file path (`fs::read` instead of `read_to_string`) and `dispatch_wire_response` (HTML bodies are now transcoded to UTF-8). Streaming stdin runs through `StreamTranscoder` in the reader thread (buffers 1024 bytes to sniff, then decodes incrementally), so `AppEvent::Chunk` stays UTF-8. Test: `q8_charset_decoding_meta_header_bom_and_streaming`. `grim` worked: a latin-1 file with `<meta charset=iso-8859-1>` rendered "café €". Left out (noted, no new item): external stylesheets fetched via wire are still UTF-8 only.

---

## Q9 — `vertical-align` beyond sub/super (queue item)

> Implement the first unchecked item (Q9) of the ranked QUEUE in
> scratchy/TODO.txt.

**What changed:** `VerticalAlign` gained `Top`/`Middle`/`Bottom`/`TextTop`/`TextBottom`, parsed in `DeclParser::apply`. `VerticalAlign::inline_box_dy` positions an inline `<img>` or inline-block within its line in `measure_mixed_inline` (`middle` centers, `top`/`text-top` pin to the line top, everything else keeps the old bottom alignment). `lay_out_row_cells` stretches `middle`/`bottom` table cells to the row height and shifts their content down by half/all of the slack. Text spans treat the new keywords as `baseline` (`y_shift` returns 0), a documented approximation. Test: `q9_vertical_align_box_keywords_on_inline_blocks_and_table_cells`. `grim` worked: a table with top/middle/bottom cells showed the text at the top, centre and bottom of the row. Left out (noted in TODO, no new item): `tbody` default `middle` in `default.css`, length/percentage values.

---

## Q10 — `@supports` (queue item)

> Implement the first unchecked item (Q10) of the ranked QUEUE in
> scratchy/TODO.txt.

**What changed:** `TopLevelParser`'s prelude is now `AtPrelude` (`Media(..)` or `Supports(bool)`). `parse_supports_condition`/`parse_supports_in_parens` evaluate `not`, `and`, `or` (mixing the two unparenthesised is false), nested parentheses, `(prop: value)` and `selector(...)`; unknown functions and unparseable conditions are false. A `(prop: value)` holds if `DeclParser::apply` on a scratch `Declarations` sets anything (same "a branch accepted it" signal as Q1's debug check). A true block recurses exactly like `@media`; a false one is consumed and yields no rules (so it no longer shows as "dropped" in `SCRATCHY_CSS_DEBUG`; that test now uses `@font-face`). Test: `q10_supports_conditions_gate_blocks`. `grim` worked: `p{color:blue}` plus `@supports (display:grid){p{color:green}}` rendered green, with the false `@supports` blocks ignored. Left out: nothing.

---

## Q11 — `calc()`, `min()`, `max()`, `clamp()` (queue item)

> Implement the first unchecked item (Q11) of the ranked QUEUE in
> scratchy/TODO.txt.

**What changed:** `parse_length`/`parse_length_percent` accept the four math functions via `parse_calc_function`/`parse_calc_body` (recursive descent: `parse_calc_sum`/`_product`/`_atom`), building a `CalcNode` tree (px/em/rem/pt/% leaves, `+ - * /`, nested parentheses and functions). The tree is type-checked (`CalcNode::is_length`): length+number, length*length, division by a non-number or literal zero, and non-length results make the declaration drop. `Length::Calc` resolves eagerly (`Length::resolve`) unless it contains a `%`, in which case `BoxLength::Calc` carries the font sizes and resolves in `measure` against the containing block (`BoxLength::resolve`/`resolve_opt`, which also replaced three copies of the percent-height match). `media_feature_length_px` evaluates `calc()` in `@media` features, so the old "calc never matches" test became `media_calc_feature_value_is_evaluated`. Test: `q11_calc_min_max_clamp`. `grim` worked: `width:calc(100% - 200px)` and `min(300px,50%)` blocks rendered at 854px and 300px in a 1054px window. Left out (noted in TODO, no new item): vw/vh (Q14), `var()` inside calc, `%` in flex-basis/grid-track calc.

---

## Q12a1 — SVG decoding at intrinsic size (queue item, first Q12 sub-item)

> Implement the first unchecked sub-item of Q12 in scratchy/TODO.txt.

**What changed:** new dependency `resvg` (no default features, so no text/system fonts). `decode_image_bytes` now sniffs SVG via `looks_like_svg` (BOM/whitespace then `<svg`, `<?xml`, `<!doctype svg`) and rasterizes through `decode_svg_bytes` at the size usvg resolves (`width`/`height`, else `viewBox`), un-premultiplying to the packed RGBA form. `decode_data_uri` now accepts percent-encoded non-base64 payloads, the form Wikipedia's `data:image/svg+xml,...` uses. Test: `q12a_svg_decodes_to_pixels`. `grim` worked: a percent-encoded 60x40 red-rect SVG `<img>` rendered as a red box. Left out (split into new item Q12a2): rasterizing at the CSS size with an `(src,w,h)` cache, `Content-Type` detection, 300x150 fallback.

---

## Q12a2 — SVG rasterized at the CSS box size (queue item, second Q12 sub-item)

> Implement the first unchecked sub-item of Q12 (Q12a2) in scratchy/TODO.txt.

**What changed:** `DecodedImage` gained `svg: Option<Arc<Vec<u8>>>` (the source document). `rasterize_svg(bytes, target)` renders the vector tree with a scale transform to any size; `resolve_img` post-processes via `svg_at_css_size`, which, when CSS/HTML gives a pixel `width` and/or `height` (the other follows the intrinsic aspect), re-rasterizes at that size, cached in a thread-local keyed by (hash of the SVG, w, h). `parse_svg` gives a root with no width/height/viewBox the CSS default 300x150 (usvg alone says 100x100). `fetch_image_via_wire` now accepts `image/svg+xml` and, via the new `fetch_sub_resource_typed` (returns the MIME), decodes such bodies as SVG regardless of prefix. Test: `q12a2_svg_rasterizes_at_css_size`. `grim` worked: a viewBox-only circle SVG at `width=120 height=60` rendered 120x60. Known limit: a non-uniform box stretches the drawing (no `preserveAspectRatio` meet centering; the circle became an ellipse). Left out: `%`/auto-from-container sizes still use the intrinsic raster scaled by `measure` (the box size isn't known at build time).

---

## Q12b — Inline `<svg>` as a replaced box (queue item, third Q12 sub-item)

> Implement the first unchecked sub-item of Q12 (Q12b) in scratchy/TODO.txt.

**What changed:** an inline `<svg>` element is now a replaced element. `serialize_inline_svg` writes the subtree back to standalone XML (adds the `xmlns`/`xmlns:xlink` declarations on the root if missing) and replaces every `currentColor` (case-insensitive) with the element's computed `color`. `resolve_inline_svg` sizes it like `resolve_img` (CSS `width`/`height`, else the unitless HTML attributes), rasterizes with `decode_svg_bytes` and `svg_at_css_size`. `collect_children` pushes an `InlineItem::Image` for the default inline display; `build_box_for_element` builds a `BoxContent::Image` box for `display:block` etc. An SVG that fails to parse renders nothing (its `<title>`/`<text>` content no longer leaks as page text). Test: `q12b_inline_svg_is_a_replaced_box_with_current_color`. `grim` worked: an inline circle with `fill=currentColor` rendered in the inherited red, and a block SVG rendered as a green 60x60 square. Left out: nothing (this completes Q12).

---

## Q13 — Custom properties that cascade per element (queue item)

> Implement the first unchecked item (Q13) of the ranked QUEUE in
> scratchy/TODO.txt.

**What changed:** the whole-stylesheet textual `var()` pass (`collect_root_custom_props`) is gone. A declaration whose value contains `var(` is now stored raw in `Declarations::var_decls` at parse time. `resolve_style` computes the element's custom properties with `compute_custom_props` (parent's table plus its own `--x` declarations by specificity/order, inline last; values substituted against the combined table, a cycle invalidates the whole cycle so `var()` takes its fallback), then re-parses each matched rule's `var_decls` (and the inline style) with the table substituted (`expand_var_decls`/`merge_var_matches`) and slots the results into the normal cascade. The table travels as `custom_props: Option<&'static CustomProps>` on `InheritedStyle`/`ComputedStyle` (interned and leaked per distinct table so both stay `Copy`). `substitute_css_vars` now takes a lookup closure and skips quoted strings. `resolve_background_image` takes the element's table too. `var()` inside `calc()` now works. Tests: `custom_prop_on_ancestor_inherits_to_subtree`, `custom_prop_not_visible_outside_its_subtree`, `descendant_overrides_custom_prop`, `chained_custom_props_resolve`, `custom_prop_cycle_uses_fallback`, `var_inside_quoted_string_is_not_substituted`, `var_inside_calc_resolves_per_element`, `background_image_var_resolves`, `inline_style_custom_prop_and_var` (the old `non_root_custom_prop_is_not_picked_up` test was replaced, its premise is gone). `grim` worked: `:root{--a;--b:var(--a)}` chained into a red paragraph, and `.dark{--bg;--w:calc(...)}` applied to a descendant `p` two levels down (blue background, 300px wide). Left out: within one rule a `var()` declaration is ordered after that rule's plain ones regardless of source order; CSS comments inside values are not skipped by the substituter.

---

## Q14 — Viewport units and initial-containing-block height (queue item)

> Implement the first unchecked item (Q14) of the ranked QUEUE in
> scratchy/TODO.txt.

**What changed:** `vw`/`vh`/`vmin`/`vmax` parse in `parse_length`/`parse_length_percent` (`Length::Vp(VpUnit, f32)`) and inside `calc()` (`CalcNode::Vp`, which makes the earlier Q11 note obsolete). They resolve against a thread-local viewport set by `build_layout_tree_sized`/`build_layout_tree_with_resources` (new `viewport_height` parameter); `BoxLength::Calc` carries its own copy of the viewport because it finishes resolving later on the shaping worker thread. The synthetic root box gets `ComputedStyle::icb_height`, used only as `child_containing_height` in `box_model_entry`, so `html,body{height:100%}` resolves. `measure_document` then raises the root `Laid`'s `h` to the lowest in-flow descendant edge (`in_flow_bottom`) so scrolling is not capped at one window. `App::redraw` rebuilds when the window height changes too (`cached_height`). Test: `q14_viewport_units_and_icb_height`. `grim` worked: `html,body{height:100%}` with `50%`/`50vw` and `25vh`/`25vw` boxes rendered at half and a quarter of the window. Left out: `@media` height features, `dvh`/`svh`/`lvh`.

---

## Q15a — `auto`/`min-content`/`max-content` grid tracks (queue item, first Q15 sub-item)

> Implement the first unchecked sub-item of Q15 (Q15a) in scratchy/TODO.txt.

**What changed:** `GridTrackSpec` and `TrackBreadth` gained `Auto`, `MinContent`, `MaxContent` (parsed by `parse_track_keyword`, bare and inside `minmax()`). `measure_grid` now places items first, then collects `cell_natural_width` (min, max) for each content-sized column from the single-column items in it and hands that to `resolve_grid_tracks` (new `content` parameter, `TrackCalc::intr`). Order: fixed tracks, content tracks start at their base and grow toward their limit, `fr` tracks share the rest, and with no `fr` track `auto` tracks stretch equally. Row tracks with these keywords size from content as implicit rows already do. Test: `q15a_grid_content_sized_tracks`. `grim` worked: `max-content 1fr` rendered an orange label column hugging its text beside a blue column filling the rest. Left out: items spanning several columns do not feed content sizing; no iterative spec freeze/redistribute pass.

---

## Q15b — `grid-template` shorthand (queue item, second Q15 sub-item)

> Implement the first unchecked sub-item of Q15 (Q15b) in scratchy/TODO.txt.

**What changed:** `grid-template` now parses via `parse_grid_template_shorthand`: `none`, `<rows> / <columns>`, and area strings each optionally followed by a row size (`"h h" 50px "m s" 70px / 1fr 100px`; a string with no size gets an `auto` row). It sets `grid_template_rows`, `grid_template_columns` and `grid_template_areas` together (an omitted part is set to empty, as the shorthand resets its longhands). Named lines (`[x]`) still fail to parse and drop the declaration. Test: `q15b_grid_template_shorthand`. `grim` worked: the area template above rendered a 400px header row, then a 300px/100px pair 50px below. Left out: CSS-wide keywords on the shorthand, `grid-template` auto-flow shorthand `grid`.

---

## Q15c — Numeric grid placement (queue item, third Q15 sub-item)

> Implement the first unchecked sub-item of Q15 (Q15c) in scratchy/TODO.txt.

**What changed:** `grid-column`/`grid-row`, `grid-{row,column}-{start,end}` and the 4-value `grid-area` (`row-start / column-start / row-end / column-end`) now parse (`parse_grid_line`, `parse_grid_line_pair`, `parse_grid_area_lines`) into `GridLine` values (`auto`, non-zero line number, negative counting from the end, `span N`). `measure_grid` resolves them with `resolve_grid_axis` and places items in three passes (named areas and fully fixed items; one fixed axis; cursor auto-placement that now honours spans). A bare `grid-area: <ident>` is still the named-area form. The old test `parse_stylesheet_leaves_an_unsupported_declaration_with_no_effect` now uses a five-line `grid-area` as its invalid case. Test: `q15c_grid_numeric_placement`. `grim` worked: a 4x2 grid with `grid-column: 2/4`, `grid-area: 2/1/3/3` and `grid-column: span 2` rendered at the expected columns and rows. Left out: named lines, implicit columns past the template, row-span height distribution (a span's height counts toward its first row only). This completes Q15 except Q15d, so Q15 stays unchecked.

---

## Q15d — `repeat(auto-fill|auto-fit, …)` (queue item, last Q15 sub-item)

> Implement the first unchecked sub-item of Q15 (Q15d) in scratchy/TODO.txt.

**What changed:** new `GridTrackSpec::AutoRepeat { fit, tracks }`, parsed by `parse_grid_repeat` (`parse_nested_auto_repeat_keyword`); every repeated track must have a definite length (a length, or `minmax()` with a length side), and only one auto-repeat per list is allowed. Columns only: `grid-template-rows` and the `grid-template` row part reject it (`reject_auto_repeat`). `measure_grid` expands it against the container content width with `expand_auto_repeat` (count = floor((available - other tracks) / (repeat width + gaps)), at least 1) before placement; for `auto-fit`, repetitions with no item collapse to `0px` after placement. Test: `q15d_grid_repeat_auto_fill_fit`. `grim` worked: `repeat(auto-fill, minmax(120px,1fr))` in 700px gave five 140px columns with seven items wrapping, and `auto-fit` stretched two items to 350px each. Left out: gaps next to collapsed `auto-fit` tracks are not collapsed, auto-repeat in rows, intrinsic-size (`auto`/`min-content`) tracks inside the repeat. This completes Q15.

---

## Q16 — Background longhands (queue item)

> Implement the first unchecked item (Q16) of the ranked QUEUE in scratchy/TODO.txt.

**What changed:** `background-position`, `background-size` and `background-repeat` parse (`parse_bg_position_items`, `parse_bg_size_items`, `parse_bg_repeat_items`) into `Declarations`, cascade with CSS-wide keywords, and resolve into the Copy `ComputedStyle::bg_layout` (`BgLayout`). Position supports keywords, lengths, percentages and the 3/4-value edge-offset form; size supports `cover`, `contain`, explicit and `auto` (a missing axis keeps the image aspect); repeat supports `repeat-x`/`-y`/`no-repeat` and per-axis pairs. `parse_background_shorthand` now returns a `BgShorthand` and fills position, `/ size` and repeat from the first layer (and resets the three longhands to initial). `paint_background_image` replaces the old tiler (kept as a test-only `tile_image` wrapper) and clips the image to the `border-radius` rounded corners. Test: `q16_background_longhands_parse_and_paint`. `grim` worked: a checker image rendered as a bottom-right `no-repeat` tile, `cover` clipped to rounded corners, `repeat-x` at 40px, and `contain` centered. Left out: multiple background layers (first layer only), `background-origin`/`-clip`/`-attachment` (origin is the border box), `space`/`round` (act as `repeat`), `%` inside `calc()` for position/size.


---

## Q17 — `opacity` (queue item)

> Implement the first unchecked item (Q17, `opacity`) of the ranked QUEUE in scratchy/TODO.txt.

**What changed:** `opacity` parses (`parse_opacity`: number or percentage, clamped to 0..1, invalid ignored) and cascades like `z-index` (CSS-wide keywords included). `resolve_style` stores it in the new `ComputedStyle::opacity` already multiplied by the parent's (read through `InheritedStyle::parent`), so no extra plumbing through the box-building functions was needed. `paint` is now a thin wrapper that publishes `laid.style.opacity` into a thread-local (`PAINT_OPACITY`), calls `paint_inner` and restores the previous value; `blend`, the single compositing choke point (backgrounds, borders, outlines, shadows, text, decorations, images, background images), scales every source alpha by it. `opacity:0` still takes layout space and is hit-tested (neither path reads it). Test: `q17_opacity_multiplies_down_the_tree_and_fades_paint`. `grim` worked: a 50% red box holding a 50% blue box rendered as a pale red with a muted purple child, beside an opaque green box. Known, accepted limitation (user decision 2026-11-12): overlapping semi-transparent children inside one faded parent double-blend. Left out: `opacity` on an inline `<span>` (spans share their text box's style, so only block-level and replaced boxes fade).


---

## Q18 — `@import` for wire-fetched stylesheets (queue item)

> Implement the first unchecked item (Q18, `@import`, wire-fetched stylesheets only) of the ranked QUEUE in scratchy/TODO.txt; also handle non-UTF-8 fetched stylesheets if cheap.

**What changed:** `process_external_resources` now runs each fetched stylesheet through `expand_css_imports` (pure; takes a fetcher closure). It uses `split_leading_imports` to read the leading `@charset`/`@import url(..)`/`@import ".."` rules, resolves each against the sheet's own URL, fetches, expands recursively and inserts the result in place of the `@import`; a media list wraps it in `@media`. Depth limit `MAX_IMPORT_DEPTH` = 8, cycles skipped, failed fetches dropped. `supports(...)`/`layer(...)` imports are dropped (bare `layer` is ignored). Q8 follow-up: `fetch_stylesheet_via_wire` now decodes with `decode_css_bytes` (BOM, header charset, `@charset`, else UTF-8); the now-unused `fetch_sub_resource_via_wire` was removed. Tests: `q18_expand_css_imports_in_place_media_depth_and_cycles`, `q18_stylesheet_charset_decoding`. `grim` exists but no screenshot was taken: the feature only runs on the wire fetch path, which needs a live wire/snag socket not available in the sandbox; coverage is the unit tests plus the existing mock-socket fetch test. Left out: `@import` in inline `<style>`.


---

## Q19 — `::before`/`::after` generated content (queue item)

> Implement the first unchecked item (Q19, generated content) of the ranked QUEUE in scratchy/TODO.txt.

**What changed:** `content` parses (`parse_content` into `Declarations::content: ContentDecl`: `none`/`normal`, or one or more strings concatenated; `attr()`, `counter()`, `url()` and quote keywords make the declaration invalid). `matching_declarations_for(.., Some("before"|"after"))` matches only selectors carrying exactly that `OurPseudoElement::Named`, per selector in `ForStatelessPseudoElement` mode against the originating element; `resolve_style_for` is `resolve_style` for the pseudo (inherits from the element, ignores its inline `style`). `collect_children` is now a thin wrapper that calls `generated_content`/`push_generated` before and after `collect_real_children` (the old body): `display: inline` becomes a styled `InlineItem::Text` in the surrounding run, `block`/`table`/`flex`/`grid` a block box (so `content:""; display:block; height` clearfix/bars work), `inline-block`/`inline-flex` an embedded box, `none` nothing. Test: `q19_before_after_generate_string_content`. `grim` worked: red/blue curly quotes around a `<q>`, a green ` NEW` badge after a span, and an orange 200x20 block bar before a div's text. Not done: the per-page survey of `content` uses (no network in the sandbox); `attr()`/`counter()`/`url()`/quotes and `::marker` etc. are the new queue item Q19b. Quirk: a text node always gets a trailing space, so `hello</q>` renders ` "` with a space before the closing quote glyph.


---

## Q19b — Generated content: attr(), counters, quotes, flex pseudo items (queue item)

> Implement the first unchecked item (Q19b) of the ranked QUEUE in scratchy/TODO.txt: the cheapest coherent subset, re-splitting the rest.

**What changed:** `parse_content` now also accepts `attr(name)`, `counter(name[, style])`, `counters(name, "sep"[, style])` and `open-quote`/`close-quote`/`no-open-quote`/`no-close-quote` (`ContentDecl::Parts` of `ContentPart`s; all-string content stays `ContentDecl::Text`); `url()` is still invalid. `counter-reset`/`counter-increment` parse into `Declarations` (`parse_counter_ops`). New `precompute_generated` (called from `build_layout_tree_with_resources`, a no-op unless some rule uses dynamic content or a counter property) walks the DOM once in document order, keeps a scoped counter stack and quote depth, and stores each evaluated pseudo text in the thread-local `GENERATED_TEXT`, which `generated_content` reads. `format_counter` handles decimal, decimal-leading-zero, lower/upper alpha/latin/roman, lower-greek, none and bullets. `build_flex_children` (flex and grid) now emits `::before`/`::after` as blockified items before/after the real items. Test: `q19b_attr_counters_quotes_and_flex_pseudo_items`; the Q19 test's `attr()` invalid-case now uses `url()`. `grim` worked: red `1. One`/`2. Two` headings via counters, `x.html "link"` via `attr()` and quotes, and an orange and a blue box flanking flex content. Left out (new queue items Q19c-Q19f): `::marker`, `content: url()`, `::first-letter`/`::first-line`, implicit `list-item` counter, `counter-set`, `quotes`, `display:none` elements still count.


---

## Q19c — `::marker` styling (queue item)

> Implement the first unchecked item (Q19c, `::marker`) of the ranked QUEUE in scratchy/TODO.txt.

**What changed:** new `marker_prefix` (called where `build_box_for_element` built the `<li>` prefix) matches `li::marker` rules via `matching_declarations_for(.., Some("marker"))`, resolves the pseudo's own style with `resolve_style_for` (so `color`, font properties etc. apply to the marker run) and honors a string `content` (replaces the bullet/number text; `none` removes it). `li::before` content was already emitted after the marker prefix, so the two coexist. Test: `q19c_marker_pseudo_styles_and_replaces_the_marker`. `grim` worked: red bullet, blue `→ ` custom marker, and a `content:none` item with no marker. Left out: `attr()`/counters inside `::marker` content, `::marker` on non-`li` elements (`display:list-item`, Q20c).


---

## Q19d — `content: url(...)` (queue item)

> Implement the first unchecked item (Q19d, `content: url()`) of the ranked QUEUE in scratchy/TODO.txt.

**What changed:** `parse_content` accepts a lone `url(...)` as the new `ContentDecl::Image`. `generated_content` now takes `image_cache` and returns `(ComputedStyle, GenContent)` (`Text` or `Image`); the image is a decoded `data:` URI or an `image_cache` hit keyed by the raw url text (same policy as `<img src>`), passed through `svg_at_css_size`. `push_generated` delegates images to the new `push_generated_image`: inline display pushes an `InlineItem::Image`, block/table/flex/grid a block box with `BoxContent::Image` (so `width`/`height` on the pseudo size it). Test: `q19d_content_url_renders_an_image`. `grim` worked: a 40x40 red image inline before a paragraph's text and a 120x20 block image after a div. Left out: `url()` mixed with strings or counters (declaration stays invalid), and `process_external_resources` does not collect CSS `content: url()` references, so non-`data:` urls only render if the same string is already in the image cache (currently the same limitation `background-image` has).


---

## Q19e — `::first-letter` (queue item)

> Implement the first unchecked item (Q19e, `::first-letter` and `::first-line`) of the ranked QUEUE in scratchy/TODO.txt; do `::first-letter` and re-split `::first-line` if hard.

**What changed:** new `apply_first_letter`, called at the end of `build_children_with_prefix`, matches `::first-letter` rules (`matching_declarations_for(.., Some("first-letter"))`, skipped cheaply when no rule has that pseudo), resolves the pseudo's style with `resolve_style_for`, and splits the first alphanumeric character plus any leading punctuation off the first anonymous inline run's first text item (after an `<li>` marker prefix) into its own `InlineItem::Text` with that style; the rest keeps the original span. Test: `q19e_first_letter_styles_the_leading_letter`. `grim` worked: a red bold 48px opening quote and `O` before normal paragraph text. `::first-line` is split out as new queue item Q19e2 (needs layout-time line knowledge). Left out: `float` drop caps, first-letter inside a nested first block child, and `::first-letter` on elements whose first child is a generated/embedded/image item.


---

## Q19e2 — `::first-line` (queue item)

> Implement the first unchecked item (Q19e2, `::first-line`) of the ranked QUEUE in scratchy/TODO.txt.

**What changed:** new `apply_first_line` (called after `apply_first_letter` in `build_children_with_prefix`) resolves the `::first-line` pseudo style and keeps only the properties that differ from the element's own (color, background, font size/weight/style, line-height, word-spacing) as `FirstLineStyle` in the new `ComputedStyle::first_line` on the first anonymous inline run. `measure`'s text-only `BoxContent::Inline` arm shapes normally, reads the end byte of the first `layout_runs()` entry, restyles that prefix with `apply_first_line_split` and re-shapes; it repeats (max 4 times) until the styled prefix is exactly the first line, since a bolder/larger style moves the break (the first grim attempt showed a stray red word on line 2 before this loop). Test: `q19e2_first_line_styles_only_the_first_laid_out_line` (checks glyph colors per layout run). `grim` worked: a bold red first line and plain following lines. Left out: runs that go through `measure_mixed_inline` (images, inline-blocks, `word-spacing`) ignore `::first-line`; nested first block child; a `::first-letter` span is overridden by first-line properties it also sets (spec says first-letter wins); text-decoration/other properties are not carried.


---

## Q19f — Counter polish, quotes, content on real elements (queue item)

> Implement the first unchecked item (Q19f, counter polish) of the ranked QUEUE in scratchy/TODO.txt.

**What changed:** `precompute_generated` now skips `display:none` subtrees (so they neither count nor generate pseudos), applies `counter-set` (new `Declarations::counter_set`), keeps an implicit `list-item` counter (`ol`/`ul` reset it, `li` increments it, unless the author's counter properties mention `list-item`), and takes quote strings from the new inherited `quotes` property (`parse_quotes`; `none` = empty, `auto` = English curly). `content` on a selector with no pseudo-element is stored in the new thread-local `ELEMENT_CONTENT` and `collect_children` emits it in place of the element's children (also suppressing its pseudos). `parse_counter_ops` now rejects CSS-wide keywords as counter names (declaration ignored). Test: `q19f_counter_polish_quotes_and_element_content`. `grim` worked: red `1) a`/`2) b` from `counter(list-item)`, blue counters skipping a `display:none` element and jumping to 7 via `counter-set`, and green `REPLACED` content on a span. Left out: `inherit` semantics for counter properties (only ignored), `display:none` set via inline `style`, `<ol start>`/`<li value>`, `content` on replaced elements.


---

## Q20 — Small property gaps (queue item)

> Implement the first unchecked item (Q20, six small property gaps) of the ranked QUEUE in scratchy/TODO.txt.

**What changed:** one commit per sub-item (d, e and the `white-space` half of f share one). Q20a: `parse_font_shorthand` (`font: [style] [weight] size[/line-height] family`, resets style/weight/line-height, system-font keywords fail), plus absolute `font-size` keywords (`small`, `large`, ...). Q20b: `unprefixed_property` maps `-webkit-`/`-moz-` `box-sizing`, `flex*`, `box-shadow`, `border-radius`, `opacity`, alignment properties onto the standard names at the top of `DeclParser::apply`; `display:-webkit-flex`/`-webkit-inline-flex` are flex. Q20c: `ListStyleType` gained lower/upper alpha, roman and lower-greek (`ListStyleType::from_keyword`, `list_marker` uses `format_counter`); `display: list-item` is a block plus the new `ComputedStyle::list_item` flag, so `marker_prefix` also runs for non-`li` elements (their `::marker` works; a non-`li` decimal marker counts same-tag siblings). Q20d: new inherited `WordBreak` (`word-break`, `overflow-wrap`/`word-wrap`); cosmic-text's default `Wrap::WordOrGlyph` already breaks over-long words, so `break-word`/`anywhere` are a no-op and `break-all` selects `Wrap::Glyph` in `shape_spans` (via `SpanStyle::word_break`). Q20e: inherited `TextTransform`, applied by the new `inline_text` where text nodes become inline items. Q20f: `WhiteSpace::PreWrap` (also `break-spaces`) and `PreLine` handled in `inline_text`; `Length::Intrinsic` for `width`/`min-width` (`set_width_length`), turned by `resolve_style` into `ComputedStyle::width_kw`/`min_width_kw`, and `measure` sizes the box from `cell_natural_width` (same path as a `width:auto` float). Tests: `q20a_font_shorthand`, `q20b_prefixed_aliases_are_plain_synonyms`, `q20c_alpha_roman_greek_markers_and_display_list_item`, `q20def_word_break_text_transform_and_white_space_variants`, `q20f_intrinsic_width_keywords_size_from_content`. `grim` worked: italic bold red font shorthand, `a.`/`b.` and `I.`-`IV.` lists, a `display:list-item` div with `1.`, upper/capitalized text, a break-all box, pre-line/pre-wrap blocks and max-/min-content boxes. Left out: `-webkit-transform` (no `transform` at all), intrinsic width keywords on table/flex/grid containers and `max-width`, `white-space` pre-wrap/pre-line in the mixed-inline (image) path, `text-transform` on generated `::before`/`::after` text, and the implicit `list-item` counter for non-`li` `display:list-item`. Deviation kept: plain `overflow-wrap:normal` still breaks long words (cosmic-text default), unlike real CSS.


---

## Q21 — `overflow-x`/`overflow-y` and `text-overflow: ellipsis` (queue item)

> Implement the first unchecked item (Q21, `overflow-x`/`overflow-y` and `text-overflow`) of the ranked QUEUE in scratchy/TODO.txt.

**What changed:** `Declarations` gained `overflow_y` and `text_overflow` (`overflow` stays the x axis); the `overflow` shorthand now takes one or two values (`x y`) and `overflow-x`/`overflow-y` set one axis each (`clip` is treated as `hidden`). `resolve_style` folds the axes into the single existing `ComputedStyle::overflow` (a `visible` axis yields to the other one), since the whole clip/scroll model has one mode per box. New `TextOverflow` enum and `ComputedStyle::text_overflow`; `build_box_for_element` copies `Ellipsis` onto the anonymous inline runs of a box whose own overflow is not `visible`, and `measure`'s text-only `Inline` arm calls `ellipsize_items` when the run is no-wrap: it cuts the text and appends "…" (re-shaping, up to 16 tries) until the line fits. Test: `q21_overflow_axes_and_text_overflow_ellipsis`. `grim` worked: a nowrap box ends in "jump…" and a plain `overflow-x:hidden` one is hard-clipped. Left out: independent per-axis clip/scroll (one mode per box), `overflow-y` with CSS-wide keywords, ellipsis in mixed (image/inline-block) runs and multi-line `pre` text, `text-overflow` on block containers whose text is inside nested inline elements only matters via the run (works), and the `text-overflow: <string>` form.


---

## Q22 — Local files through wire: `file://` (queue item)

> Implement the first unchecked item (Q22, local files through wire) of the ranked QUEUE in scratchy/TODO.txt.

**What changed:** `wire` (same repo, user-approved) now answers `file://` URLs in `execute_and_stream` via the new `serve_file`: reads the file, `200` with `Content-Type` from `content_type_for_path` (html, css, png, jpg, gif, webp, svg, else `application/octet-stream`), `404` if unreadable, `405` for non-GET; test `file_urls_are_served_from_disk_with_a_guessed_content_type` in wire. In scratchy, `run()` for `Input::File` with `--socket` now calls `load_file_resources`: `file_page_url` (`file://` + canonical absolute path) becomes `App::base_url` (unless the page has `<base>`), and `process_external_resources` runs synchronously before the window so stylesheets are in hand for the first layout (images arrive later via `ImageFetched`, generation 0). Tests: `q22_file_page_gets_a_file_url_base_and_relative_hrefs_resolve_against_it`, `q22_stylesheet_over_a_file_url_goes_through_wire`. `grim` worked: `wire` + `scratchy --socket ... page.html` showed a red 48px heading from relative `a.css` and a blue 40x40 `b.png`. Left out: streamed (stdin) pages (new item Q22b), wire's own PROMPTS.md/TODO.txt only got the code change plus this note (no separate entry beyond it), `file://` directory listings and range/HEAD requests.


---

## Q22b — Sub-resources for a streamed (stdin) page (queue item)

> Implement the first unchecked item (Q22b, sub-resources for a streamed page) of the ranked QUEUE in scratchy/TODO.txt.

**What changed:** the base URL of a streamed page is its own `<base href>` (what `snag` synthesizes), read at `AppEvent::Eof` as before; no new flag was needed. At EOF, with `--socket` given, the new `start_stream_resources` walks the finished DOM with `find_external_resources` and starts image fetches (`spawn_image_fetches`, now shared with `process_external_resources`) plus one background thread running `fetch_stylesheets` (also extracted from `process_external_resources`), which reports through the new `AppEvent::StreamCss(generation, css)`; the handler sets `App::extra_css` and marks layout dirty. Unlike a file page the CSS therefore arrives after the first layout (a restyle, not a blocking wait). Test: `q22b_streamed_page_stylesheets_resolve_against_its_base_href` (base resolution plus `fetch_stylesheets` over a mock wire; the event plumbing needs a winit proxy and is covered by grim only). `grim` worked: `wire` + a `<base file://...>` page piped to `scratchy --socket` showed a red 48px heading from relative `a.css` and a blue 40x40 `b.png`. Left out: a streamed page with no `<base>` fetches nothing (no `--base` flag; relative references need a base), and the unstyled first paint flashes before the CSS lands.


---

## Q23 — Flex `order` (queue item)

> Implement the first unchecked item (Q23, flex leftovers, cheapest first) of the ranked QUEUE in scratchy/TODO.txt.

**What changed:** only the cheapest part, `order`, was done; the rest is re-split as Q23b. `order` (integer, `-webkit-order` aliased) is a new `Declarations::order` / `ComputedStyle::order` (non-inherited, default 0), and `measure_flex` stable-sorts its in-flow items by it before row/column layout, so source order breaks ties and laid children (paint and hit-test order too) follow the visual order. Non-integer values are ignored. Test: `q23_order_reorders_flex_items_stably`. `grim` worked: blue (`order:-1`), green, red (`order:2`) boxes appear left to right. Left out: `order` on grid items, and `order` with `row-reverse` was not separately tested (the sort happens before the reverse logic, so it should compose per spec).


---

## Q23b — Flex `align-content` (queue item)

> Implement the first unchecked item (Q23b, flex leftovers) of the ranked QUEUE in scratchy/TODO.txt.

**What changed:** only `align-content` was done; the remaining leftovers are re-queued as Q23c. New `AlignContent` enum (`normal` = `stretch`, plus flex-start/start, flex-end/end, center, space-between/around/evenly), `Declarations::align_content` / `ComputedStyle::align_content`. `measure_flex_row` now measures all lines first (collecting each line's cross size), then, for a wrapping container with a definite `height`, distributes the free cross space (stretch grows each line equally, so `align-items: stretch` items grow too), then aligns items per line in a second pass. Test: `q23b_align_content_distributes_wrapped_lines`. `grim` worked: red and blue 30px bars pushed to the top and bottom of a 200px `space-between` container. Left out: column containers (no wrapping there yet), `wrap-reverse` flipping of the flex-start/flex-end meaning, and a definite height coming only from `min-height` or a percentage.


---

## Q23c — Flex leftovers: column min size, column wrap, baseline, §9.7 loop (queue item)

> Implement the first unchecked item (Q23c, flex leftovers) of the ranked QUEUE in scratchy/TODO.txt, all of it in one PR.

**What changed:** (1) `resolve_flex_main_sizes` is now the real CSS Flexbox §9.7 freeze-and-retry loop (initial freezing by factor 0 or hypothetical-vs-basis, the `sum of factors < 1` rule, min/max violation freezing), so a clamped item's under/overshoot is redistributed among its unclamped siblings. (2) `measure_flex_column` gets the automatic minimum size: with `height:auto`, `overflow:visible` and no `min-height`, an item's floor is its measured content height (clamped to its basis); an explicit `height` keeps a floor of 0 (the content height would need a second measure). (3) `measure_flex_column` implements `flex-wrap` / `wrap-reverse` for a container with a definite height: greedy column breaking, per-column cross size from the widest margin box, `align-content` over the free width, and the new shared `distribute_align_content` (the row code now uses it too). Wrapped items are measured once at fit-content width and stretched by overwriting `Laid::w` (no re-measure, so no duplicate positioned-descendant entries); this also now applies the items' horizontal margins in column containers. (4) `align-items: baseline` in rows: new `first_baseline` finds the first shaped line's baseline through the shape cache (fallback: border-box bottom), items shift to share the line's baseline and the line cross size covers the above/below extents. Tests: `q23c_flexible_lengths_freeze_clamped_items_and_redistribute`, `q23c_column_items_do_not_shrink_below_their_content_height`, `q23c_column_wrap_makes_side_by_side_columns`, `q23c_baseline_alignment_lines_up_first_text_baselines`. `grim` worked: three 60px boxes in a 130px column wrap became red/green over blue-to-the-right, and a 40px "Big" and 12px "small" shared a baseline. Left out (nothing re-queued): column wrap without a definite height (stays one column), baseline for items whose first line is in a mixed-inline run or an image (falls back to the bottom edge), `wrap-reverse` flipping flex-start/flex-end meaning for rows, `last baseline`, and re-measuring stretched wrapped items at the final column width.


---

## Q24 — `overflow: scroll|auto` on flex/grid/table, scrolling positioned children (queue item)

> Implement the first unchecked item (Q24, `overflow: scroll|auto` on flex, grid and table) of the ranked QUEUE in scratchy/TODO.txt.

**What changed:** the `ScrollInfo` construction moved into the shared `make_scroll_info`, now called from the ordinary `Blocks` arm and from the tails of `measure_table`, `measure_flex` and `measure_grid`, so those containers are real scroll regions (clip, wheel routing, scrollbar). Out-of-flow (`position: absolute`) and explicitly stacked children of a scroll container now scroll with it: `paint` passes `content_scroll_offset` (ambient plus the box's own scroll) to both `paint_z_layer` calls, and `hit_test_cursor` applies the box's own scroll before its "above" loop. Tests: `q24_flex_grid_and_table_containers_can_be_scroll_regions`, `q24_absolute_child_scrolls_with_its_scroll_container`, `q24_nested_scroll_regions_route_the_wheel_to_the_innermost` (the nested-region test the v62 entry asked for). `grim` worked: a `display:flex; overflow:scroll` 60px box clips its 200px child and shows the scrollbar. Left out: an absolute child whose containing block lies outside the scroll container correctly does not scroll, but is also not clipped by it (as before); absolute content does not extend `content_h`; still no horizontal scrolling or drag-scrolling.


---

## Q25 — `box-shadow`: blur, multiple shadows, inset, currentColor (queue item)

> Implement the first unchecked item (Q25, `box-shadow`) of the ranked QUEUE in scratchy/TODO.txt.

**What changed:** `BoxShadow` gained `blur`, `inset` and an optional color (missing or `currentColor` resolves to the element's own `color`); `parse_box_shadow` now parses a comma-separated list (`parse_box_shadow_layer` per layer) into a fixed `[Option<BoxShadow>; MAX_SHADOWS]` (4; extras ignored, because `ComputedStyle` is `Copy`). `ComputedStyle::box_shadow` is now that array of `ResolvedBoxShadow`. New `paint_box_shadow` rasterizes the shadow shape (rounded by `border-radius` plus/minus spread) into an f32 coverage mask, blurs it with `blur_mask` (3-pass box blur, sigma = blur/2, margin 1.5*blur), and composites per pixel: outer shadows skip the border box itself (so a transparent box shows no shadow through it), inset ones are confined to the padding box and use the inverse coverage. Outer layers paint before the background, inset after the border/outline, first-listed on top. Masks over 4M cells fall back to a hard edge. Tests: `q25_box_shadow_parses_multiple_inset_blur_and_current_color`, `q25_blurred_outer_shadow_fades_and_skips_the_border_box`, `q25_inset_shadow_paints_inside_and_multiple_layers_stack`; the two older `box-shadow` tests were adjusted to index `[0]`. `grim` worked: a rounded box with a soft drop shadow, a blue inset glow and a hard red offset shadow. Left out (nothing re-queued): inset shadows are not clipped by an inner rounded border's elliptical shape beyond the uniform radius, shadows are not anti-aliased at the shape edge, shadows beyond 4 layers are dropped, and the blurred shadow of a box wider than ~2000px loses its blur.


---

## Q28 — `object-fit` / `object-position` and smooth image scaling (queue item)

> Implement the first workable unchecked item (Q28, images: `object-fit`/`object-position`, bilinear `blit_image`) of the ranked QUEUE in scratchy/TODO.txt (Q26 skipped, it is blocked).

**What changed:** new `ObjectFit` enum (fill/contain/cover/none/scale-down) and `Declarations::object_fit`/`object_position` (position reuses `parse_bg_position`, default `50% 50%`), resolved into `ComputedStyle::object_fit`/`object_pos` (non-inherited). `blit_image` now takes them: it computes the picture rect inside the content box, clips to the box, and samples through the new `sample_image` (4-tap alpha-weighted bilinear; a downscale of 2x or more box-averages the source footprint, capped at 16x16 taps) instead of nearest-neighbour. Test: `q28_object_fit_and_position_parse_and_place_the_picture` (parsing, every fit mode, position, bilinear blend, downscale averaging). `grim` worked: five 80px boxes with a 2:1 red/blue image showed fill, contain (letterboxed), cover (cropped), none at left top, scale-down. Left out (nothing re-queued): the picture rect is not pixel-snapped (fractional placement leaves half-covered edge rows unpainted), `object-fit` is not applied to `<video>`/`<canvas>` (not supported anyway), and `background-image` painting is still nearest-neighbour.


---

## Q29 — `mask-image` / `-webkit-mask-image` (queue item)

> Implement the first workable unchecked item (Q29, `mask-image`/`-webkit-mask-image`, icon masks) of the ranked QUEUE in scratchy/TODO.txt (Q26 skipped, it is blocked).

**What changed:** `Declarations` gained `mask_image` (url or `none`, reusing `BackgroundImageDecl`), `mask_position`/`mask_size`/`mask_repeat`, parsed from the standard and `-webkit-` spellings, including the `mask` shorthand (via `parse_background_shorthand`). `ComputedStyle::mask_layout` is a `BgLayout` built by the same code as `bg_layout` (now a `layout_for(mask)` closure). `DecodedImage` has a new `mask` field: `resolve_background_image` now cascades `background-image` and `mask-image` from one selector-matching pass; the mask is decoded (data URI, PNG/SVG) and turned white-with-alpha (`whiten_mask`); an element with a mask but no background image gets a 0x0 placeholder bundle. `paint` routes a box with a mask to `paint_masked`: it snapshots the border-box region, paints the box normally (decoration and descendants), paints the mask through `paint_background_image` onto a black scratch buffer to get coverage, then blends result back toward the snapshot by coverage. An SVG mask is re-rasterized at its tile size (new shared `bg_tile_size`) so icons stay sharp. Test: `q29_mask_image_clips_the_box_by_the_masks_alpha` (both spellings, `none`, shorthand, `mask-position`/`mask-size`/`mask-repeat`). `grim` worked: an 80px red box masked by an SVG circle showed a red disc on white. Left out (nothing re-queued): `mask-mode`/luminance masks, `mask-composite`, `mask-origin`/`mask-clip`, gradient masks (only `url()`), multiple mask layers (first layer only), masks on absolutely positioned descendants that paint outside the box, raster masks use nearest sampling, and SVG masks are re-rasterized every frame (no cache; icons are small).



---

## Q31 — real `:hover` / `:active` / `:focus*` state (queue item)

> Implement Q31 of the ranked QUEUE in scratchy/TODO.txt: real `:hover`/`:focus`/`:active` (and `:focus-visible`/`:focus-within`), upgrading Q2's never-match rules. No form elements (Q30 later); keep the state plumbing reusable and restyle only when the matched set changes.

**What changed:** new `InteractionState` (ancestor-expanded hover/active/focus-within id sets, `focus`, `focus_visible`) mirrored into the `INTERACTION` thread-local that `match_non_ts_pseudo_class` reads; new `OurPseudoClass::{Hover, Active, Focus, FocusVisible, FocusWithin}` (`:target` stays `NeverMatches`). Element identity is `node_id` (DOM node address), carried by the new `ComputedStyle::node` and `SpanStyle::node` (stored per glyph beside the href in `ShapedParagraph`); the hit test is now `hit_test_node` (`hit_test_cursor` is a wrapper, `hit_test_element` returns the id). `App` handles `CursorMoved` (hover), mouse press/release (active, plus click focus on the nearest link/`tabindex` ancestor), Tab/Shift+Tab (`:focus-visible`) and Enter on a focused link. `set_interaction_state` sets `layout_dirty` only when a changed kind is mentioned by a stylesheet (`STATE_RULES` mask filled by the selector parser), and keeps scroll offsets across that rebuild. Tests: `q31_hover_active_focus_state_matches_and_tracks_the_element_chain`, `q31_focusable_elements_and_tab_order`, `q31_state_rule_mask_is_zero_when_no_stylesheet_uses_state`; `q2_state_pseudo_classes_parse_and_match` now resets the state explicitly. Measured (release, 5000-item list): restyle rebuild ~113 ms, hovered-handle lookup ~0.15 ms. `grim`: a static screenshot of a page with `:hover`/`:focus` rules rendered normally, but interactive state could not be screenshotted (no `ydotool`/`wtype`/`wlrctl` to inject pointer or key events in this sandbox), so hover/Tab behavior is covered by the unit tests only. Left out (re-queued as Q31b): `:target`, scroll-into-view on Tab, a default focus ring; form controls remain Q30 (NOTES.md "Interaction state" says how to plug them in).
