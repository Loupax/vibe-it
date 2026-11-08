# wire — working notes

## What this is for

`scratchy` has a firm, long-standing design stance: it does no
networking of its own (see `scratchy/NOTES.md`, "rendering, not
networking"). Fetching has always been the caller's job — originally
exactly one caller, `snag`, which bundles a whole page (HTML +
stylesheets + images) into one self-contained file for `scratchy` to
parse. That model works great for "render one page I already fetched,"
and that's still `snag`'s entire job.

It breaks down the moment `scratchy` needs to react to something the
*user* does inside a running session — clicking a link, submitting a
form. Those need a real, individual HTTP request fired on demand, not a
whole-page bundle built ahead of time. `scratchy/TODO.txt`'s v65 item
bolted this onto `snag` directly (`snag --serve`, a line-protocol daemon
that takes a URL and returns a bundle) as a first cut, but a follow-up
design conversation (logged in `scratchy/TODO.txt`'s "TOP PRIORITY"
entry) decided that was the wrong shape going forward: `snag`'s own job
is bundling, not being a general-purpose HTTP client for `scratchy` to
drive interactively, and conflating the two was already showing seams
(a GET-only line protocol with no way to express a form's method/body/
headers).

`wire` is the fix: a tool with exactly one job, smaller than either of
its siblings, that owns real networking and nothing else. `scratchy`
still does zero networking. `snag` still does zero live interactivity.
`wire` is the one place an actual TCP/TLS connection to a real remote
server gets opened.

## Design choices

- **Proxy absolute-form request framing (`GET https://host/path
  HTTP/1.1`), not a bespoke protocol.** `wire` sits behind a bare Unix
  socket with no DNS/TLS-SNI of its own to infer a destination from —
  something has to tell it exactly where to connect. Standard HTTP
  proxy framing (RFC 7230 §5.3.2) already solves this with zero
  invented syntax, and it's symmetric with the daemon's own job
  description ("acts like an HTTP proxy, minus the TCP tunnel"). Using
  it also means `scratchy`'s own request-building code can reuse
  exactly the same shape for a GET link click and a GET/POST form
  submission — see `scratchy/TODO.txt`'s own "forms and link-clicks
  become the exact same code path" decision.

- **A normalized response, never a raw 1:1 relay — the one point
  actually debated before this tool existed at all** (see
  `scratchy/TODO.txt`'s own writeup of the conversation). The
  alternative — just forwarding the real server's bytes untouched —
  would leave chunked-decoding/redirect-following/gzip-decoding as new
  HTTP-client logic `scratchy` would have to grow itself, directly
  contradicting its "no networking" stance; the *transport* mess is
  real HTTP's problem to solve once, in one place, not scratchy's to
  re-solve per feature. So `wire` always reads the real response to
  completion — redirects already followed, body already decompressed —
  and re-frames it with a correct `Content-Length`, stripping
  `Transfer-Encoding`/`Content-Encoding` (both now meaningless: the
  body in hand is already the final, decoded bytes). Everything else
  about *what the bytes mean* (is this HTML? an image? something
  scratchy can't render at all?) stays scratchy's decision, made by
  reading exactly one header, `Content-Type` — `wire` doesn't try to
  guess or collapse different content kinds into one shape.

- **A real non-2xx is not a transport failure.** `ureq` represents
  these as two different error-shaped outcomes under the hood
  (`Err(Error::Status(code, response))` carries a genuine response;
  `Err(Error::Transport(e))` means no response ever came back at all),
  and `execute` treats them accordingly: a `Status` error goes through
  the *exact same* `normalize_response` path as a plain `Ok` — a 404
  forwards as a 404, not folded into some generic "it failed" shape —
  while only a real `Transport` failure (DNS, connect, TLS handshake,
  timeout) produces the synthesized `502`. Getting this distinction
  backwards would have been an easy, quiet bug: naively matching
  `Err(_) => synthetic_error_response(...)` compiles fine and passes a
  test that never happens to hit a real 404, which is exactly why one
  of this file's own tests (`a_real_4xx_from_upstream_forwards_as_itself_
  not_a_transport_error`) exists — to pin the distinction, not just the
  happy path.

- **One thread per connection, not a serial accept loop.** `snag
  --serve`'s own loop (serve one connection, then accept the next) was
  the obvious first precedent to reach for, but the TODO item that
  specified this daemon is explicit that concurrent, independent
  connections are an expected shape (a page with several forms/links'
  worth of future per-resource fetches, each its own one-shot
  connection) — not a hypothetical. `std::thread::spawn` per accepted
  connection, with `ureq::Agent` cloned into each (it's an `Arc` handle
  internally, so cloning is cheap and thread-safe), is the simplest
  thing that actually satisfies that requirement; nothing fancier (a
  thread pool, async I/O) is justified until a real page's connection
  count makes raw `thread::spawn` itself the bottleneck, which nothing
  here has shown yet.

- **Only `gzip` decompression, not `brotli` too.** Real CSS/browser
  precedent treats gzip as the dominant real-world `Content-Encoding`
  by a wide margin, and `ureq`'s `brotli` feature is a separate,
  additional dependency with its own compile cost — not justified
  before something real actually needs it, the same per-need-only
  philosophy this repo already applies everywhere else (see e.g.
  `scratchy/TODO.txt`'s own `box-shadow`/`gap`-percentage items for the
  same reasoning pattern). Easy to add later; `is_response_header_to_
  strip`/`normalize_response` don't care which encoding produced the
  decompressed bytes they're framing.

- **No request-side chunked support.** The one real client is
  `scratchy`, which always knows its own request body's length up front
  (a form's URL-encoded value string, built in memory, has a length
  the instant it exists) — so `Content-Length`-framed request bodies
  are the only case worth handling. Nothing currently needs a daemon
  that can *receive* a chunked request.

- **Response headers otherwise pass through verbatim, unfiltered.**
  `wire` doesn't try to decide which upstream headers matter —
  deciding that is explicitly scratchy's job (reading `Content-Type`
  and nothing else, per the TODO item's own contract). Stripping
  headers here beyond the three transport-framing ones above would be
  `wire` quietly making a product decision on scratchy's behalf.

## What's deliberately not here yet

See `wire/TODO.txt` for the precise, maintained list — this section is
just the short version. The daemon itself (parse → execute → normalize
→ write) is complete and tested for what it's scoped to do; what's
missing is entirely on the *consuming* side: `scratchy` doesn't dial
`wire` yet (still talks to `snag --serve`), and `snag --serve` itself
hasn't been removed now that `wire` supersedes it for live navigation.
Both are explicitly the next slice of the same `scratchy/TODO.txt` item
this tool was built to satisfy, not a gap in `wire` itself.
