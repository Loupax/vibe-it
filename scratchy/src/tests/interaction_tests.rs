#[allow(unused_imports)]
use super::*;

// ---------- v65: link navigation ----------

// -- href resolution against the page's own <base href> --

#[test]
fn resolve_href_relative_joins_against_base() {
    let base = Url::parse("https://example.com/articles/foo").unwrap();
    let resolved = resolve_href(Some(&base), "bar").unwrap();
    assert_eq!(resolved.as_str(), "https://example.com/articles/bar");
}

#[test]
fn resolve_href_absolute_ignores_base_entirely() {
    // Real `Url::join` behavior, same as a real browser: an already-
    // absolute href resolves to itself regardless of what base the
    // page declares.
    let base = Url::parse("https://example.com/articles/foo").unwrap();
    let resolved = resolve_href(Some(&base), "https://other.example/bar").unwrap();
    assert_eq!(resolved.as_str(), "https://other.example/bar");
}

#[test]
fn resolve_href_relative_with_no_base_is_a_clear_error() {
    let err = resolve_href(None, "relative/path").unwrap_err();
    assert!(
        err.contains("no <base href>"),
        "error should explain there's nothing to resolve a relative link against: {err}"
    );
}

#[test]
fn resolve_href_absolute_with_no_base_still_resolves() {
    // No `<base>` on the page at all doesn't block an *already*-
    // absolute href — only a relative one has nothing to join
    // against.
    let resolved = resolve_href(None, "https://example.com/standalone").unwrap();
    assert_eq!(resolved.as_str(), "https://example.com/standalone");
}

#[test]
fn classify_input_recognizes_http_and_https_urls() {
    assert!(matches!(classify_input("http://example.com"), Input::Url(_)));
    assert!(matches!(classify_input("https://example.com/wiki/Foo"), Input::Url(_)));
}

#[test]
fn classify_input_treats_everything_else_as_a_file_path() {
    assert!(matches!(classify_input("page.html"), Input::File(_)));
    assert!(matches!(classify_input("/home/user/page.html"), Input::File(_)));
    // A scheme scratchy has no `wire`-backed fetch path for — same
    // "parsed successfully isn't the same question as fetchable"
    // reasoning `classify_input`'s own docs give.
    assert!(matches!(classify_input("data:text/plain,hi"), Input::File(_)));
}

#[test]
fn qnav9a_file_url_argument_opens_the_local_path() {
    match classify_input("file:///etc/hosts") {
        Input::File(p) => assert_eq!(p, "/etc/hosts"),
        _ => panic!("file URL should be Input::File"),
    }
    // Percent-escapes are decoded; `localhost` is local.
    match classify_input("file://localhost/tmp/a%20b/c%C3%A9.html") {
        Input::File(p) => assert_eq!(p, "/tmp/a b/cé.html"),
        _ => panic!("file URL should be Input::File"),
    }
    assert!(file_url_error("file:///etc/hosts").is_none());
    assert!(file_url_error("page.html").is_none());
    assert!(file_url_error("https://example.com/").is_none());
    let err = file_url_error("file://fileserver/share/x.html").expect("non-local host rejected");
    assert!(err.contains("fileserver") && err.contains("not supported"), "{err}");
}

#[test]
fn qnav9b_without_wire_a_relative_file_link_loads_the_local_file() {
    let dir = std::env::temp_dir().join(format!("scratchy-qnav9b-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a b.html");
    let b = dir.join("next.html");
    std::fs::write(&a, "<html><body><a href=\"next.html\">n</a></body></html>").unwrap();
    std::fs::write(&b, "<html><body><p>second</p></body></html>").unwrap();
    let base = file_page_url(a.to_str().unwrap()).expect("file url");
    let target = resolve_href(Some(&base), "next.html").unwrap();
    let page = read_local_file_page(&target).expect("local file read");
    assert!(String::from_utf8_lossy(&page.html).contains("second"));
    assert_eq!(page.url, target);
    // A missing file is a clear message, not a panic.
    let missing = resolve_href(Some(&base), "gone.html").unwrap();
    let err = read_local_file_page(&missing).err().expect("missing file errors");
    assert!(err.contains("gone.html"), "{err}");
    // A plain-text file is wrapped, not parsed as HTML.
    std::fs::write(dir.join("t.txt"), "<b>x</b>").unwrap();
    let txt = read_local_file_page(&resolve_href(Some(&base), "t.txt").unwrap()).unwrap();
    assert!(String::from_utf8_lossy(&txt.html).contains("&lt;b&gt;"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn find_base_href_finds_a_real_base_tag() {
    let doc = parse_html(
        r#"<html><head><base href="https://example.com/articles/"></head><body>hi</body></html>"#,
    );
    assert_eq!(find_base_href(&doc), Some("https://example.com/articles/".to_string()));
}

#[test]
fn find_base_href_is_none_when_the_page_has_no_base_tag() {
    let doc = parse_html("<html><head></head><body>hi</body></html>");
    assert_eq!(find_base_href(&doc), None);
}

// -- v68: a fetched page with no <base> of its own falls back to the
// real URL it was fetched from, instead of leaving base_url at None
// and breaking every relative link on that page (found live — see
// PROMPTS.md's own v68 entry) --

#[test]
fn resolve_page_base_url_prefers_the_pages_own_base_tag_when_present() {
    let doc = parse_html(r#"<html><head><base href="https://example.com/real/"></head><body>hi</body></html>"#);
    let fetched_url = Url::parse("https://example.com/different-url").unwrap();
    assert_eq!(resolve_page_base_url(&doc, fetched_url).as_str(), "https://example.com/real/");
}

#[test]
fn resolve_page_base_url_falls_back_to_the_fetched_url_when_the_page_has_no_base_tag() {
    // The exact real-world shape that broke live: a page with no
    // <base> at all (the common case) must still let a relative —
    // including a same-page fragment, e.g. MediaWiki's own
    // "#bodyContent" skip-link — link resolve, by falling back to
    // wherever the page was actually fetched from.
    let doc = parse_html("<html><head></head><body><a href=\"#bodyContent\">skip</a></body></html>");
    let fetched_url = Url::parse("https://en.wikipedia.org/wiki/Example").unwrap();
    let base = resolve_page_base_url(&doc, fetched_url.clone());
    assert_eq!(base, fetched_url);
    // And confirms the actual downstream effect: a relative href on
    // this page now resolves at all, instead of erroring with "no
    // <base href> on this page to resolve it against".
    assert_eq!(
        resolve_href(Some(&base), "#bodyContent").unwrap().as_str(),
        "https://en.wikipedia.org/wiki/Example#bodyContent"
    );
}

// -- click-vs-drag distinction --

#[test]
fn is_click_accepts_a_tiny_move_between_down_and_up() {
    // Ordinary hand tremor between a real mouse-down and mouse-up
    // over the same link shouldn't be mistaken for a drag.
    assert!(is_click((100.0, 100.0), (102.0, 101.0), CLICK_DRAG_THRESHOLD_PX));
}

#[test]
fn is_click_rejects_a_real_drag() {
    assert!(!is_click((100.0, 100.0), (260.0, 100.0), CLICK_DRAG_THRESHOLD_PX));
}

#[test]
fn is_click_boundary_is_inclusive() {
    // Exactly at the threshold distance still counts as a click, not
    // a drag — consistent with this file's other boundary-inclusive
    // conventions (see e.g. `@media`'s own min-width tests).
    assert!(is_click((0.0, 0.0), (CLICK_DRAG_THRESHOLD_PX, 0.0), CLICK_DRAG_THRESHOLD_PX));
}

// -- href recovered at glyph precision (reuses `cursor`'s own v32/
// v61 hit-testing machinery — same `hit_test_cursor` function, now
// returning the href half of its result too) --

#[test]
fn hit_test_cursor_recovers_the_real_href_of_a_clicked_link() {
    // `default.css` already declares `a { cursor: pointer }` (v32),
    // so `find_glyph_center`'s existing `CursorStyle::Pointer` lookup
    // doubles as "find the link's own glyph" here with no new test
    // helper needed.
    let (laid, cache) = measure_fixture_with_cache(
        r#"<html><body><p><a href="/page">link text</a> plain text</p></body></html>"#,
        400.0,
    );
    let (lx, ly) = find_glyph_center(&laid, &cache, CursorStyle::Pointer).expect("no link glyph found");
    let (px, py) = find_glyph_center(&laid, &cache, CursorStyle::Auto).expect("no plain-text glyph found");
    let scroll_offsets = HashMap::new();
    assert_eq!(
        hit_test_cursor(&laid, lx, ly, &cache, &scroll_offsets, 0.0, 600.0).1,
        Some("/page"),
        "a glyph inside the link should report its real href"
    );
    assert_eq!(
        hit_test_cursor(&laid, px, py, &cache, &scroll_offsets, 0.0, 600.0).1,
        None,
        "an adjacent plain-text glyph should report no href at all"
    );
}

#[test]
fn href_ambiently_propagates_into_a_nested_element_with_no_href_of_its_own() {
    // Not a CSS property (see `SpanStyle::href`'s own docs), so this
    // isn't real CSS inheritance — but a `<b>` nested inside a real
    // `<a href>`, with no `href` of its own, still needs to report
    // the ancestor link's href, the same ambient-propagation shape
    // `background-color`/`text-decoration` already use.
    let (laid, cache) = measure_fixture_with_cache(
        r#"<html><body><p><a href="/x"><b>bold link</b></a></p></body></html>"#,
        400.0,
    );
    let (bx, by) = find_glyph_center(&laid, &cache, CursorStyle::Pointer).expect("no link glyph found");
    let scroll_offsets = HashMap::new();
    assert_eq!(hit_test_cursor(&laid, bx, by, &cache, &scroll_offsets, 0.0, 600.0).1, Some("/x"));
}

#[test]
fn an_anchor_with_no_href_is_not_a_link() {
    // A fragment-target `<a name="...">`/`<a>` with no `href` at all
    // is real HTML, not a link — same distinction `OurPseudoClass::
    // Link`'s own `:link` matching already makes (v37). Note this
    // doesn't mean no cursor: `default.css`'s `a { cursor: pointer
    // }` rule is a plain tag selector, not `:link`-scoped, so it
    // still applies here — `find_glyph_center` has to look for
    // `Pointer`, same as a real link, even though `href` itself must
    // come back `None`. This is exactly why `href` needs its own
    // tracking rather than piggybacking on `cursor`.
    let (laid, cache) = measure_fixture_with_cache(
        r#"<html><body><p><a>not a link</a></p></body></html>"#,
        400.0,
    );
    let (x, y) = find_glyph_center(&laid, &cache, CursorStyle::Pointer).expect("no glyph found");
    let scroll_offsets = HashMap::new();
    assert_eq!(hit_test_cursor(&laid, x, y, &cache, &scroll_offsets, 0.0, 600.0).1, None);
}

// -- QNAV9: a click anywhere on a link follows it, not only on its text glyphs --

/// The glyph href and the href `enclosing_link_href` finds from the element under `(x, y)`.
fn qnav9_link_at(html: &str, x: f32, y: f32) -> (Option<&'static str>, Option<String>) {
    let doc = parse_html(html);
    let tree = build_layout_tree(&doc, 1920.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let laid = measure(
        &tree, &mut font_system, 0.0, 0.0, 400.0, None, &mut next_id, &mut cache, deadline, &mut incomplete,
        &mut Vec::new(), &Floats::default(), None,
    );
    let (_, glyph_href, node) = hit_test_node(&laid, x, y, &cache, &HashMap::new(), 0.0, 600.0);
    let from_node = (node != 0).then(|| find_node_by_id(&doc, node)).flatten().and_then(|h| enclosing_link_href(&h));
    (glyph_href, from_node)
}

#[test]
fn qnav9_blank_area_of_a_block_link_resolves_to_its_href() {
    let html = r#"<html><body style="margin:0"><a href="other.html" style="display:block;height:80px;background:#fc8">x</a></body></html>"#;
    // Far right of the 80 px tall block: no glyph there, so the glyph hit test has no href.
    let (glyph, node) = qnav9_link_at(html, 300.0, 60.0);
    assert_eq!(glyph, None, "no text under the point");
    assert_eq!(node.as_deref(), Some("other.html"), "the enclosing <a href> is still the click target");
}

#[test]
fn qnav9_image_inside_a_link_resolves_to_its_href() {
    let html = r##"<html><body style="margin:0"><a href="#frag"><img src="x.png" width=100 height=60 style="display:block"></a></body></html>"##;
    let (_, node) = qnav9_link_at(html, 50.0, 30.0);
    assert_eq!(node.as_deref(), Some("#frag"));
}

#[test]
fn qnav9_outside_a_link_and_anchor_without_href_are_not_links() {
    let html = r#"<html><body style="margin:0"><a name="n" style="display:block;height:80px">x</a><p>plain</p></body></html>"#;
    let (_, node) = qnav9_link_at(html, 300.0, 40.0);
    assert_eq!(node, None, "<a> without href is not a link");
    let (_, node) = qnav9_link_at(html, 300.0, 95.0);
    assert_eq!(node, None);
}

#[test]
fn qnav9_hrefs_of_a_file_page_resolve_against_the_file_base() {
    let base = Url::parse("file:///home/u/site/pages/index.html").unwrap();
    let r = |h: &str| resolve_href(Some(&base), h).unwrap().to_string();
    assert_eq!(r("other.html"), "file:///home/u/site/pages/other.html");
    assert_eq!(r("../x.html"), "file:///home/u/site/x.html");
    assert_eq!(r("#frag"), "file:///home/u/site/pages/index.html#frag");
    assert_eq!(r("file:///tmp/a.html"), "file:///tmp/a.html");
    assert_eq!(r("https://example.org/"), "https://example.org/");
}

// -- building the absolute-form request `wire` itself parses --

#[test]
fn build_get_request_text_uses_absolute_form_and_a_bare_host_header() {
    let url = Url::parse("https://example.com/path?q=1").unwrap();
    assert_eq!(
        build_get_request_text(&url),
        "GET https://example.com/path?q=1 HTTP/1.1\r\nHost: example.com\r\n\r\n"
    );
}

#[test]
fn build_get_request_text_includes_an_explicit_port_in_the_host_header() {
    let url = Url::parse("http://example.com:8080/x").unwrap();
    assert_eq!(build_get_request_text(&url), "GET http://example.com:8080/x HTTP/1.1\r\nHost: example.com:8080\r\n\r\n");
}

// -- parsing wire's own normalized response shape --

#[test]
fn parse_wire_response_extracts_content_type_and_body() {
    let raw = b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 5\r\n\r\nhello";
    let (content_type, body) = parse_wire_response(raw).expect("parse");
    assert_eq!(content_type, Some("text/html".to_string()));
    assert_eq!(body, b"hello");
}

#[test]
fn parse_wire_response_content_type_lookup_is_case_insensitive_and_ignores_other_headers() {
    let raw = b"HTTP/1.1 200 OK\r\ncontent-type: image/png\r\nServer: wire\r\n\r\n\x89PNG";
    let (content_type, body) = parse_wire_response(raw).expect("parse");
    assert_eq!(content_type, Some("image/png".to_string()));
    assert_eq!(body, b"\x89PNG");
}

#[test]
fn parse_wire_response_errors_on_a_missing_header_terminator() {
    assert!(parse_wire_response(b"not a real response at all").is_err());
}

// -- dispatching by Content-Type, per `scratchy/TODO.txt`'s own
// settled response-framing contract --

#[test]
fn q8_charset_decoding_meta_header_bom_and_streaming() {
    // windows-1252 bytes, <meta charset=iso-8859-1> -> "é" (0xE9).
    let mut doc = b"<meta charset=iso-8859-1><p>caf".to_vec();
    doc.push(0xE9);
    assert!(decode_html_bytes(&doc, None).ends_with("caf\u{e9}"));
    // http-equiv form, and windows-1252 specific 0x80 = euro sign.
    let doc2 = b"<meta http-equiv=\"Content-Type\" content=\"text/html; charset=windows-1252\">\x80";
    assert!(decode_html_bytes(doc2, None).ends_with('\u{20ac}'));
    // Header charset wins over meta; wire path transcodes to UTF-8.
    let out = dispatch_wire_response(Some("text/html; charset=ISO-8859-1".to_string()), vec![b'a', 0xE9]).unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), "a\u{e9}");
    // BOM'd UTF-8 has the BOM stripped; no BOM/meta stays UTF-8.
    assert_eq!(decode_html_bytes(b"\xEF\xBB\xBFhi \xC3\xA9", None), "hi \u{e9}");
    assert_eq!(decode_html_bytes("h\u{e9}".as_bytes(), None), "h\u{e9}");
    // Streaming: split chunks, sniff buffered, multi-byte split ok.
    let mut t = StreamTranscoder::new();
    let mut out = t.decode(b"<meta charset=latin1>", false);
    assert!(out.is_empty());
    out.extend(t.decode(&[0xE9; 1500], false));
    out.extend(t.decode(&[], true));
    let s = String::from_utf8(out).unwrap();
    assert!(s.starts_with("<meta") && s.ends_with("\u{e9}") && s.matches('\u{e9}').count() == 1500);
    // Short stream flushed at EOF.
    let mut t = StreamTranscoder::new();
    assert!(t.decode(b"<meta charset=latin1>\xE9", false).is_empty());
    assert!(String::from_utf8(t.decode(&[], true)).unwrap().ends_with('\u{e9}'));
}

#[test]
fn dispatch_wire_response_passes_html_through_untouched() {
    let body = b"<html><body>hi</body></html>".to_vec();
    assert_eq!(dispatch_wire_response(Some("text/html; charset=utf-8".to_string()), body.clone()), Ok(body));
}

#[test]
fn dispatch_wire_response_wraps_a_real_image_in_a_synthetic_html_document() {
    // A real tiny PNG, built the same way `tiny_png_data_uri` builds
    // one for the pre-existing `<img>`-rendering tests just above —
    // decoded back out of its own data: URI purely to get the raw
    // bytes a real `wire` response body would carry. Round-tripped
    // through the *real* `decode_data_uri` this produces an
    // `<img src>` for — not just a string-shape check — to confirm
    // the wrapped document is actually renderable, not merely
    // well-formed-looking HTML.
    let png_bytes = decode_data_uri(&tiny_png_data_uri(1, 1)).expect("decode the fixture's own data: URI");
    let html = dispatch_wire_response(Some("image/png".to_string()), png_bytes.clone()).expect("dispatch");
    let html = String::from_utf8(html).expect("utf8 html");
    assert!(html.starts_with("<html><body><img src=\"data:image/png;base64,"), "got: {html}");
    let data_uri_start = html.find("src=\"").unwrap() + "src=\"".len();
    let data_uri_end = html[data_uri_start..].find('"').unwrap() + data_uri_start;
    let decoded = decode_data_uri(&html[data_uri_start..data_uri_end]).expect("decode_data_uri");
    assert_eq!(decoded, png_bytes, "the wrapped data: URI must decode back to the exact original bytes");
}

#[test]
fn dispatch_wire_response_errors_on_an_unsupported_content_type() {
    let err = dispatch_wire_response(Some("application/pdf".to_string()), vec![1, 2, 3]).unwrap_err();
    assert!(err.contains("application/pdf"), "error should name the unsupported content type: {err}");
}

#[test]
fn qb7_text_plain_is_wrapped_in_an_escaped_pre() {
    let out = dispatch_wire_response(Some("text/plain; charset=utf-8".to_string()), b"a<b\n  c".to_vec()).unwrap();
    let html = String::from_utf8(out).unwrap();
    assert!(html.contains("a&lt;b\n  c</pre>"), "{html}");
    let out = dispatch_wire_response(Some("text/plain; charset=ISO-8859-1".to_string()), vec![b'a', 0xE9]).unwrap();
    assert!(String::from_utf8(out).unwrap().contains("a\u{e9}</pre>"));
    let out = dispatch_wire_response(Some("text/plain".to_string()), b"\nx".to_vec()).unwrap();
    assert!(String::from_utf8(out).unwrap().contains("<pre style=\"white-space:pre-wrap\">\n\nx"));
}

#[test]
fn dispatch_wire_response_errors_clearly_when_there_is_no_content_type_at_all() {
    assert!(dispatch_wire_response(None, vec![1, 2, 3]).is_err());
}

#[test]
fn fetch_raw_via_wire_round_trips_a_real_response_over_a_real_unix_socket() {
    use std::os::unix::net::UnixListener;
    let path = unique_test_socket_path("roundtrip");
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).expect("bind mock wire socket");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut received = Vec::new();
        // The client always closes its write side (or just drops
        // the connection after reading) once it has what it wants —
        // reading to EOF here mirrors exactly what the real `wire`
        // daemon's own `handle_connection` does on its side of this
        // same protocol.
        let mut buf = [0u8; 4096];
        loop {
            match Read::read(&mut stream, &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    received.extend_from_slice(&buf[..n]);
                    if received.ends_with(b"\r\n\r\n") {
                        break; // a GET request has no body to wait for
                    }
                }
            }
        }
        assert_eq!(
            received,
            build_get_request_text(&Url::parse("https://example.com/page").unwrap()).into_bytes(),
            "scratchy must send the real absolute-form request wire itself parses"
        );
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 29\r\n\r\n<html><body>hi</body></html>")
            .expect("write response");
        // Dropping `stream` here closes the connection — the real
        // `wire` protocol's own "done" signal (see `wire/src/
        // main.rs`'s own `handle_connection`).
    });
    let url = Url::parse("https://example.com/page").unwrap();
    let request_text = build_get_request_text(&url);
    let result = fetch_raw_via_wire(&path, &request_text);
    server.join().expect("mock server thread panicked");
    let _ = std::fs::remove_file(&path);
    match result {
        WireFetchRaw::Responded { content_type, body, .. } => {
            assert_eq!(content_type, Some("text/html".to_string()));
            assert_eq!(body, b"<html><body>hi</body></html>");
        }
        WireFetchRaw::Unreachable => panic!("expected a successful response, got Unreachable"),
    }
}

#[test]
fn fetch_raw_via_wire_reports_unreachable_when_nothing_is_listening() {
    let path = unique_test_socket_path("missing");
    let _ = std::fs::remove_file(&path);
    let request_text = build_get_request_text(&Url::parse("https://example.com/").unwrap());
    assert!(matches!(fetch_raw_via_wire(&path, &request_text), WireFetchRaw::Unreachable));
}

// -- sub-resource fetching (v67): external stylesheets and images --

#[test]
fn find_external_resources_collects_stylesheet_links_and_non_data_img_srcs() {
    let doc = parse_html(
        r#"<html><head>
            <link rel="stylesheet" href="a.css">
            <link rel="icon" href="favicon.ico">
            <link rel="alternate stylesheet" href="b.css">
        </head><body>
            <img src="photo.jpg">
            <img src="data:image/png;base64,iVBORw0KGgo=">
        </body></html>"#,
    );
    let mut resources = ExternalResources::default();
    find_external_resources(&doc, &mut resources);
    assert_eq!(resources.stylesheets, vec!["a.css".to_string(), "b.css".to_string()]);
    assert_eq!(resources.images, vec!["photo.jpg".to_string()]);
}

#[test]
fn q22_file_page_gets_a_file_url_base_and_relative_hrefs_resolve_against_it() {
    let dir = std::env::temp_dir().join(format!("scratchy-q22-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let page = dir.join("page.html");
    std::fs::write(&page, "<html></html>").unwrap();
    let base = file_page_url(page.to_str().unwrap()).expect("file url");
    assert_eq!(base.scheme(), "file");
    assert!(base.path().ends_with("/page.html"));
    let css = resolve_href(Some(&base), "style/a.css").unwrap();
    assert_eq!(css.scheme(), "file");
    assert!(css.path().ends_with("/style/a.css"));
    // The request `wire` receives is plain absolute-form.
    assert!(build_get_request_text(&css).starts_with("GET file:///"));
    assert!(file_page_url("/nonexistent/q22.html").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn q22_stylesheet_over_a_file_url_goes_through_wire() {
    let path = unique_test_socket_path("q22-css");
    let _ = std::fs::remove_file(&path);
    spawn_mock_wire_once(&path, b"HTTP/1.1 200 OK\r\nContent-Type: text/css\r\n\r\np{margin:0}");
    let url = Url::parse("file:///tmp/a.css").unwrap();
    assert_eq!(fetch_stylesheet_via_wire(&path, &url, &url), Some("p{margin:0}".to_string()));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn q22b_streamed_page_stylesheets_resolve_against_its_base_href() {
    let path = unique_test_socket_path("q22b-css");
    let _ = std::fs::remove_file(&path);
    spawn_mock_wire_once(&path, b"HTTP/1.1 200 OK\r\nContent-Type: text/css\r\n\r\nh1{color:red}");
    let doc = parse_html("<html><head><base href=\"file:///tmp/site/\"><link rel=stylesheet href=a.css></head><body><h1>x</h1></body></html>");
    let base = find_base_url(&doc).unwrap();
    let mut res = ExternalResources::default();
    find_external_resources(&doc, &mut res);
    assert_eq!(resolve_href(Some(&base), &res.stylesheets[0]).unwrap().as_str(), "file:///tmp/site/a.css");
    assert_eq!(fetch_stylesheets(&path, &base, &base, &res.stylesheets), "h1{color:red}\n");
    // And a page with no <base> has nothing to resolve against.
    assert!(find_base_url(&parse_html("<link rel=stylesheet href=a.css>")).is_none());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn fetch_stylesheet_via_wire_only_accepts_real_css() {
    let path = unique_test_socket_path("css-ok");
    let _ = std::fs::remove_file(&path);
    spawn_mock_wire_once(&path, b"HTTP/1.1 200 OK\r\nContent-Type: text/css\r\nContent-Length: 15\r\n\r\nbody{color:red}");
    let url = Url::parse("https://example.com/style.css").unwrap();
    // A real connection, over a real socket — not a direct call into
    // `fetch_sub_resource_via_wire`'s own internals — the same
    // "real I/O, not just the pure logic" bar `fetch_raw_via_wire`'s
    // own test already holds to.
    assert_eq!(fetch_stylesheet_via_wire(&path, &url, &url), Some("body{color:red}".to_string()));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn q18_expand_css_imports_in_place_media_depth_and_cycles() {
    let base = Url::parse("https://e.com/css/main.css").unwrap();
    let mut fetched: Vec<String> = Vec::new();
    let mut fetch = |u: &Url| -> Option<String> {
        fetched.push(u.to_string());
        match u.path() {
            "/css/a.css" => Some("@import 'b.css';\na{color:red}".to_string()),
            "/css/b.css" => Some("@import url(main.css);\nb{color:blue}".to_string()),
            "/css/p.css" => Some("p{margin:0}".to_string()),
            _ => None,
        }
    };
    let css = "@charset \"utf-8\";\n/* c */ @import url(\"a.css\");\n@import p.css;\n@import \"p.css\" print and (min-width:1px);\n@import \"missing.css\";\nbody{x:y}";
    let mut stack = vec![base.clone()];
    let out = expand_css_imports(css, &base, &mut stack, &mut fetch);
    // `@import p.css` (unquoted) is not valid CSS and is dropped.
    let b = out.find("b{color:blue}").unwrap();
    let a = out.find("a{color:red}").unwrap();
    let m = out.find("@media print and (min-width:1px) {").unwrap();
    assert!(b < a && a < m && m < out.find("body{x:y}").unwrap());
    assert!(!out.contains("@import"));
    // The cycle back to main.css was skipped, not fetched.
    assert!(!fetched.iter().any(|u| u.ends_with("main.css")));
    // Depth limit: a self-chaining fetcher stops.
    let mut n = 0;
    let mut chain = |_: &Url| -> Option<String> {
        n += 1;
        Some(format!("@import \"x{n}.css\"; i{n}{{}}"))
    };
    let out = expand_css_imports("@import \"x0.css\";", &base, &mut vec![base.clone()], &mut chain);
    assert_eq!(out.matches("{}").count(), MAX_IMPORT_DEPTH - 1);
}

#[test]
fn q18_stylesheet_charset_decoding() {
    assert_eq!(decode_css_bytes(b"a{content:'\xE9'}", Some(encoding_rs::WINDOWS_1252)), "a{content:'é'}");
    assert_eq!(decode_css_bytes(b"@charset \"iso-8859-1\"; a{content:'\xE9'}", None), "@charset \"iso-8859-1\"; a{content:'é'}");
    assert_eq!(decode_css_bytes(b"\xEF\xBB\xBFa{}", None), "a{}");
}

#[test]
fn fetch_stylesheet_via_wire_rejects_a_non_css_content_type() {
    let path = unique_test_socket_path("css-wrong-type");
    let _ = std::fs::remove_file(&path);
    spawn_mock_wire_once(&path, b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 4\r\n\r\n<p/>");
    let url = Url::parse("https://example.com/not-css").unwrap();
    assert_eq!(fetch_stylesheet_via_wire(&path, &url, &url), None);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn fetch_image_via_wire_decodes_a_real_fetched_png() {
    use base64::Engine;
    let data_uri = tiny_png_data_uri(2, 2);
    let payload = data_uri.strip_prefix("data:image/png;base64,").unwrap();
    let png_bytes = base64::engine::general_purpose::STANDARD.decode(payload).unwrap();
    let mut raw = format!("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\n\r\n", png_bytes.len()).into_bytes();
    raw.extend_from_slice(&png_bytes);
    let path = unique_test_socket_path("image-ok");
    let _ = std::fs::remove_file(&path);
    spawn_mock_wire_once(&path, raw.leak());
    let url = Url::parse("https://example.com/pixel.png").unwrap();
    let decoded = fetch_image_via_wire(&path, &url, &url).expect("decode");
    assert_eq!((decoded.width, decoded.height), (2, 2));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn fetch_image_via_wire_rejects_a_non_image_content_type() {
    let path = unique_test_socket_path("image-wrong-type");
    let _ = std::fs::remove_file(&path);
    spawn_mock_wire_once(&path, b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\nhello");
    let url = Url::parse("https://example.com/not-an-image").unwrap();
    assert!(fetch_image_via_wire(&path, &url, &url).is_none());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn resolve_img_renders_a_cached_live_fetched_image_for_a_non_data_uri_src() {
    let doc = parse_html(r#"<html><body><img src="https://example.com/photo.jpg" style="display:block"></body></html>"#);
    let img_handle = find_element_by_tag(&doc, "img").expect("no <img> found in fixture");
    let decoded = DecodedImage { width: 4, height: 4, pixels: vec![TextColor::rgb(1, 2, 3); 16], svg: None, mask: None };
    let mut image_cache = HashMap::new();
    image_cache.insert("https://example.com/photo.jpg".to_string(), Arc::new(decoded));
    let (_, found) = resolve_img(&img_handle, resolve_style(&img_handle, &[], ROOT_STYLE, 1920.0), &image_cache);
    assert!(found.is_some(), "a src present in image_cache must be used, not fall back to alt text");
    let found = found.unwrap();
    assert_eq!((found.width, found.height), (4, 4));
}

#[test]
fn resolve_img_is_a_cache_miss_for_a_non_data_uri_src_not_yet_fetched() {
    let doc = parse_html(r#"<html><body><img src="https://example.com/not-fetched-yet.jpg"></body></html>"#);
    let img_handle = find_element_by_tag(&doc, "img").expect("no <img> found in fixture");
    let (_, found) = resolve_img(&img_handle, resolve_style(&img_handle, &[], ROOT_STYLE, 1920.0), &HashMap::new());
    assert!(found.is_none(), "an empty image_cache must be a clean miss, not a panic or a guess");
}

#[test]
fn build_layout_tree_with_resources_applies_extra_css_to_the_real_cascade() {
    let doc = parse_html(r#"<html><body><div class="box">x</div></body></html>"#);
    let extra_css = ".box { background-color: #00ff00; }";
    let tree = build_layout_tree_with_resources(&doc, 1920.0, None, extra_css, &HashMap::new());
    assert_eq!(
        find_box_with_background(&tree),
        Some(TextColor::rgb(0, 0xff, 0)),
        "CSS fetched from an external stylesheet must cascade exactly like an inline <style> block's own rules"
    );
}

#[test]
fn build_layout_tree_with_resources_renders_a_cached_image_as_a_real_image_box() {
    let doc = parse_html(r#"<html><body><img src="https://example.com/photo.jpg" style="display:block"></body></html>"#);
    let mut image_cache = HashMap::new();
    image_cache.insert(
        "https://example.com/photo.jpg".to_string(),
        Arc::new(DecodedImage { width: 2, height: 2, pixels: vec![TextColor::rgb(9, 9, 9); 4], svg: None, mask: None }),
    );
    let tree = build_layout_tree_with_resources(&doc, 1920.0, None, "", &image_cache);
    fn find_image_box(b: &LayoutBox) -> Option<&LayoutBox> {
        if matches!(b.content, BoxContent::Image(_)) {
            return Some(b);
        }
        if let BoxContent::Blocks(children) = &b.content {
            for c in children {
                if let Some(found) = find_image_box(c) {
                    return Some(found);
                }
            }
        }
        None
    }
    assert!(
        find_image_box(&tree).is_some(),
        "a non-data: <img> with its src already in image_cache must render as a real image box, not fall back to alt text"
    );
}

// `process_external_resources` itself isn't unit-tested directly —
// it takes a real `EventLoopProxy<AppEvent>`, and this file's own
// established precedent (see NOTES.md's v65/v62/v55/v32 entries) is
// to never construct a real winit `EventLoop` inside `#[cfg(test)]`
// just to get one; its two real pieces (`find_external_resources`,
// `fetch_stylesheet_via_wire`, `fetch_image_via_wire`, all above)
// are each independently covered, and the thin orchestration between
// them (resolve each href, fetch/spawn) rests on code review plus
// the live interactive verification `PROMPTS.md`'s own v67 entry
// describes, same category as `start_navigation`'s own click-side
// wiring already is.

// -- `App::show_message`'s HTML-escaping, since it echoes back
// arbitrary page/OS-controlled text (a clicked href, an io::Error)
// into synthetic markup --

#[test]
fn escape_html_escapes_the_characters_that_would_corrupt_the_synthetic_markup() {
    assert_eq!(escape_html("a < b & c > d"), "a &lt; b &amp; c &gt; d");
}

// ---------- vim mode: `resolve_binding`/`TextEditBuffer` ----------
// (TODO.txt's own plan — the mode enum, real keyboard wiring, the
// `:` command bar, `:set vim`/`:set novim`, `:go`, `:help`, `:q`/
// `:quit`, Tab completion, and the `SIGUSR1` bootstrap for turning
// vim mode back on once off.)

// ---------- command bar: Tab completion, `:q`/`:quit`/`:help`/`:go` ----------

#[test]
fn resolve_binding_maps_bare_jk_to_scroll_in_normal_mode() {
    let key = Key::Character("j".into());
    assert_eq!(resolve_binding(&key, ModifiersState::empty(), VimMode::Normal), Some(Action::ScrollDown));
    let key = Key::Character("k".into());
    assert_eq!(resolve_binding(&key, ModifiersState::empty(), VimMode::Normal), Some(Action::ScrollUp));
}

#[test]
fn resolve_binding_requires_control_for_half_page_scroll() {
    let key = Key::Character("d".into());
    assert_eq!(resolve_binding(&key, ModifiersState::empty(), VimMode::Normal), None);
    assert_eq!(
        resolve_binding(&key, ModifiersState::CONTROL, VimMode::Normal),
        Some(Action::HalfPageDown)
    );
}

#[test]
fn resolve_binding_colon_enters_command_mode() {
    let key = Key::Character(":".into());
    assert_eq!(resolve_binding(&key, ModifiersState::empty(), VimMode::Normal), Some(Action::EnterCommandMode));
}

#[test]
fn resolve_binding_maps_bare_hl_to_history_back_and_front() {
    let key = Key::Character("h".into());
    assert_eq!(resolve_binding(&key, ModifiersState::empty(), VimMode::Normal), Some(Action::HistoryBack));
    let key = Key::Character("l".into());
    assert_eq!(resolve_binding(&key, ModifiersState::empty(), VimMode::Normal), Some(Action::HistoryFront));
}

#[test]
fn resolve_binding_maps_capital_g_and_home_end_to_scroll_extremes() {
    let n = VimMode::Normal;
    let m = ModifiersState::empty();
    assert_eq!(resolve_binding(&Key::Character("G".into()), m, n), Some(Action::ScrollBottom));
    assert_eq!(resolve_binding(&Key::Named(NamedKey::End), m, n), Some(Action::ScrollBottom));
    assert_eq!(resolve_binding(&Key::Named(NamedKey::Home), m, n), Some(Action::ScrollTop));
    assert_eq!(resolve_binding(&Key::Character("G".into()), m, VimMode::Command), None);
    // bare `g` alone is not a binding; it goes through `resolve_pending_g`.
    assert_eq!(resolve_binding(&Key::Character("g".into()), m, n), None);
}

#[test]
fn pending_g_double_g_scrolls_top_and_other_keys_clear() {
    let m = ModifiersState::empty();
    let g = Key::Character("g".into());
    assert_eq!(resolve_pending_g(false, &g, m), (None, true, true));
    assert_eq!(resolve_pending_g(true, &g, m), (Some(Action::ScrollTop), false, true));
    for other in [Key::Character("j".into()), Key::Named(NamedKey::Escape), Key::Character(":".into())] {
        assert_eq!(resolve_pending_g(true, &other, m), (None, false, false));
        assert_eq!(resolve_pending_g(false, &other, m), (None, false, false));
    }
    // `g`, `j`, `g` must not fire: the `j` cleared the flag.
    let (_, p, _) = resolve_pending_g(false, &g, m);
    let (_, p, _) = resolve_pending_g(p, &Key::Character("j".into()), m);
    assert_eq!(resolve_pending_g(p, &g, m), (None, true, true));
    // Ctrl-g is not `g`.
    assert_eq!(resolve_pending_g(false, &g, ModifiersState::CONTROL), (None, false, false));
}

#[test]
fn action_ignores_repeat_exempts_scroll_but_not_history_navigation() {
    // The regression this closes: holding `h` fires `Action::
    // HistoryBack` on every OS key-repeat tick, and `navigate_
    // history` isn't gated on its own fetch completing, so an
    // un-gated repeat would race straight through the whole
    // history stack before a single page even loads.
    assert!(action_ignores_repeat(Action::HistoryBack));
    assert!(action_ignores_repeat(Action::HistoryFront));
    assert!(!action_ignores_repeat(Action::ScrollDown));
    assert!(!action_ignores_repeat(Action::ScrollUp));
    assert!(!action_ignores_repeat(Action::HalfPageDown));
    assert!(!action_ignores_repeat(Action::HalfPageUp));
}

#[test]
fn resolve_binding_never_fires_outside_normal_mode() {
    let key = Key::Character("j".into());
    assert_eq!(resolve_binding(&key, ModifiersState::empty(), VimMode::Command), None);
}

#[test]
fn resolve_binding_ignores_unbound_keys() {
    let key = Key::Character("x".into());
    assert_eq!(resolve_binding(&key, ModifiersState::empty(), VimMode::Normal), None);
}

#[test]
fn key_insertion_text_handles_the_named_space_key_not_just_character_keys() {
    // The exact regression reported post-merge: `winit` reports
    // space as `Key::Named(NamedKey::Space)`, so a match arm that
    // only handles `Key::Character` silently drops it.
    assert_eq!(key_insertion_text(&Key::Named(NamedKey::Space)), Some(" "));
    assert_eq!(key_insertion_text(&Key::Character("a".into())), Some("a"));
    assert_eq!(key_insertion_text(&Key::Named(NamedKey::Enter)), None);
    assert_eq!(key_insertion_text(&Key::Named(NamedKey::Escape)), None);
}

#[test]
fn text_edit_buffer_insert_backspace_and_move_track_a_real_cursor() {
    let mut buf = TextEditBuffer::default();
    buf.insert('v');
    buf.insert('i');
    buf.insert('m');
    assert_eq!(buf.text, "vim");
    assert_eq!(buf.cursor, 3);

    buf.move_left();
    buf.move_left();
    assert_eq!(buf.cursor, 1);

    buf.insert('!');
    assert_eq!(buf.text, "v!im");
    assert_eq!(buf.cursor, 2);

    buf.backspace();
    assert_eq!(buf.text, "vim");
    assert_eq!(buf.cursor, 1);
}

#[test]
fn text_edit_buffer_backspace_and_move_left_at_start_are_no_ops() {
    let mut buf = TextEditBuffer::default();
    buf.backspace();
    buf.move_left();
    assert_eq!(buf.text, "");
    assert_eq!(buf.cursor, 0);
}

#[test]
fn text_edit_buffer_move_right_at_end_is_a_no_op() {
    let mut buf = TextEditBuffer::default();
    buf.insert('x');
    buf.move_right();
    assert_eq!(buf.cursor, 1);
}

#[test]
fn text_edit_buffer_moves_by_whole_codepoints_not_raw_bytes() {
    let mut buf = TextEditBuffer::default();
    buf.insert('é'); // 2 UTF-8 bytes, one char
    assert_eq!(buf.cursor, 2);
    buf.move_left();
    assert_eq!(buf.cursor, 0);
    buf.backspace(); // no-op at start, must not panic on a mid-codepoint byte index
    assert_eq!(buf.text, "é");
}

#[test]
fn text_edit_buffer_insert_str_drops_control_characters() {
    // The exact real-world shape this guards against: a clipboard
    // paste ending in a trailing newline, which `App::
    // paste_into_command_line` hands straight to `insert_str`.
    let mut buf = TextEditBuffer::default();
    buf.insert_str("https://example.com/\n");
    assert_eq!(buf.text, "https://example.com/");
    assert_eq!(buf.cursor, buf.text.len());
}

#[test]
fn text_edit_buffer_insert_str_inserts_at_the_cursor_not_just_the_end() {
    let mut buf = TextEditBuffer::default();
    buf.insert_str("go https://b/");
    buf.cursor = 3; // right after "go "
    buf.insert_str("https://a/ ");
    assert_eq!(buf.text, "go https://a/ https://b/");
}

#[test]
fn token_start_before_finds_the_last_whitespace_boundary() {
    assert_eq!(token_start_before("go", 2), 0, "single token: starts at 0");
    assert_eq!(token_start_before("set vi", 6), 4, "second token starts right after the space");
    assert_eq!(token_start_before("set ", 4), 4, "cursor right after a trailing space: empty token there");
    assert_eq!(token_start_before("", 0), 0);
}

#[test]
fn command_completions_offers_command_names_for_the_first_token() {
    let candidates = command_completions("g", 0, 1);
    assert_eq!(candidates, vec!["go"]);
}

#[test]
fn command_completions_offers_set_args_only_for_sets_own_second_token() {
    let candidates = command_completions("set v", 4, 5);
    assert_eq!(candidates, vec!["vim"]);
}

#[test]
fn command_completions_has_nothing_to_offer_for_an_arbitrary_argument() {
    // `:go`'s own URL argument — no finite candidate pool exists for
    // it, by design (see `command_completions`'s own docs).
    assert_eq!(command_completions("go ht", 3, 5), Vec::<&str>::new());
}

#[test]
fn command_completions_prefix_matches_only_not_substrings() {
    // "o" is a substring of "go" but not a prefix of it — must not
    // match.
    assert_eq!(command_completions("o", 0, 1), Vec::<&str>::new());
}

/// DIAGNOSTIC, not a real regression test (no behavior assertion
/// beyond "didn't run out of the deadline") — kept permanently,
/// `#[ignore]`d, as the measured evidence `measure_mixed_inline`'s
/// own docs cite for why it shapes a whole text item at once rather
/// than one `Buffer` per word. Compares the *same total text* shaped
/// two ways: as plain paragraphs, and as the same paragraphs each
/// with one small inline image dropped in the middle (real-world
/// trigger: Wikipedia's own inline icons). Run with `cargo test
/// --release mixed_inline_overhead_diagnostic -- --ignored --nocapture`.
#[test]
#[ignore]
fn mixed_inline_overhead_diagnostic() {
    let src = tiny_png_data_uri(4, 4);
    let words_per_paragraph = 40;
    let paragraph_count = 300;
    let word = "lorem ";

    let mut plain_html = String::from("<!doctype html><html><body>\n");
    let mut mixed_html = String::from("<!doctype html><html><body>\n");
    for _ in 0..paragraph_count {
        plain_html.push_str("<p>");
        mixed_html.push_str("<p>");
        for w in 0..words_per_paragraph {
            plain_html.push_str(word);
            mixed_html.push_str(word);
            if w == words_per_paragraph / 2 {
                mixed_html.push_str(&format!(r#"<img src="{src}" style="width: 12px; height: 12px">"#));
            }
        }
        plain_html.push_str("</p>\n");
        mixed_html.push_str("</p>\n");
    }
    plain_html.push_str("</body></html>");
    mixed_html.push_str("</body></html>");

    let time_it = |html: &str| -> std::time::Duration {
        let doc = parse_html(html);
        // Wide enough that a 20-word chunk (half of each paragraph,
        // split by the image at the midpoint) comfortably fits on
        // one line regardless of font metrics — isolating the
        // common "sparse icon in flowing prose" case from the rare
        // "chunk itself wraps" fallback this function's own docs
        // also handle.
        let tree = build_layout_tree(&doc, 5000.0);
        let mut font_system = new_font_system();
        let mut cache = ShapeCache::new();
        let mut next_id = 0usize;
        let mut incomplete = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        let start = std::time::Instant::now();
        measure_document(&tree, &mut font_system, 5000.0, 5000.0, &mut next_id, &mut cache, deadline, &mut incomplete);
        let elapsed = start.elapsed();
        assert!(!incomplete, "diagnostic deadline too short");
        elapsed
    };

    let plain_time = time_it(&plain_html);
    let mixed_time = time_it(&mixed_html);
    eprintln!(
        "plain: {:?} ({} paragraphs, {} words each, no inline images)",
        plain_time, paragraph_count, words_per_paragraph
    );
    eprintln!(
        "mixed: {:?} ({} paragraphs, {} words each, one inline image per paragraph)",
        mixed_time, paragraph_count, words_per_paragraph
    );
    eprintln!("ratio (mixed / plain): {:.2}x", mixed_time.as_secs_f64() / plain_time.as_secs_f64());
}

/// DIAGNOSTIC, not a real regression test — kept permanently,
/// `#[ignore]`d, alongside `mixed_inline_overhead_diagnostic` as the
/// measured evidence for the same doc comment. Isolates `shape_
/// spans`'s own per-call overhead directly: N calls, one word each
/// (the shape of `measure_mixed_inline`'s old per-word path), versus
/// 1 call with all N words as a single span (its current whole-
/// chunk path) — same font system, same total text, same function.
/// Run with `cargo test --release
/// shape_spans_per_call_overhead_diagnostic -- --ignored --nocapture`.
#[test]
#[ignore]
fn shape_spans_per_call_overhead_diagnostic() {
    let word_count = 12_000; // matches 300 paragraphs * 40 words in the other diagnostic
    let span = SpanStyle::from(ROOT_STYLE);
    let mut font_system = new_font_system();

    let start = std::time::Instant::now();
    for _ in 0..word_count {
        let items = [InlineItem::Text("lorem ".to_string(), span)];
        shape_spans(&mut font_system, &items, 1_000_000.0, true, None);
    }
    let per_word_time = start.elapsed();

    let one_big_text = "lorem ".repeat(word_count);
    let start = std::time::Instant::now();
    let items = [InlineItem::Text(one_big_text, span)];
    shape_spans(&mut font_system, &items, 1_000_000.0, true, None);
    let one_call_time = start.elapsed();

    eprintln!("{word_count} separate 1-word shape_spans calls: {per_word_time:?}");
    eprintln!("1 shape_spans call with all {word_count} words: {one_call_time:?}");
    eprintln!("ratio: {:.2}x", per_word_time.as_secs_f64() / one_call_time.as_secs_f64());
}

/// Regression test for a real hang: a span with `line-height: 0`
/// (legitimate, real-world CSS — e.g. Wikipedia's own thumbnail-figure
/// styling) used to make `shape_spans`'s call to cosmic-text's
/// `Buffer::shape_until_scroll` loop forever, because cosmic-text
/// never returns for a zero line height. `shape_spans` now floors the
/// line height just above zero before handing it to cosmic-text. Runs
/// the call on a background thread with a watchdog so a regression
/// fails loudly (panic) instead of hanging the whole test suite.
#[test]
fn shape_spans_zero_line_height_does_not_hang() {
    let mut span = SpanStyle::from(ROOT_STYLE);
    span.line_height = 0.0;
    let items = [InlineItem::Text("Map".to_string(), span)];

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut font_system = new_font_system();
        shape_spans(&mut font_system, &items, 1_000_000.0, true, None);
        let _ = tx.send(());
    });
    rx.recv_timeout(std::time::Duration::from_secs(5))
        .expect("shape_spans hung with line_height == 0.0");
}

// ---------- QA6: fragment of the first page load ----------

/// Lays out `app.document` (so node ids match) and delivers it as a
/// `ShapeResult` for the current layout generation.
fn qa6_deliver_layout(app: &mut App, incomplete: bool) {
    let tree = build_layout_tree(&app.document, 400.0);
    let mut fc = new_font_system();
    let mut cache = ShapeCache::new();
    let (mut next_id, mut inc) = (0usize, false);
    let laid = measure(
        &tree, &mut fc, 0.0, 0.0, 400.0, None, &mut next_id, &mut cache,
        std::time::Instant::now() + std::time::Duration::from_secs(5), &mut inc, &mut Vec::new(),
        &Floats::default(), None,
    );
    let generation = app.layout_generation;
    app.handle_shape_result(generation, Box::new(laid), cache, incomplete);
}

#[test]
fn qa6_fragment_waits_for_real_page_and_target() {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = match EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build() {
        Ok(l) => l,
        Err(_) => return, // no display available (headless CI)
    };
    let mut app = App::new_static(parse_html("<html></html>"), event_loop.create_proxy(), None);
    app.fetch_url(Url::parse("http://127.0.0.1:9/p#sec").unwrap());
    // The "Loading…" page lays out first: it must not consume the fragment.
    qa6_deliver_layout(&mut app, false);
    assert_eq!(app.nav_fragment.as_deref(), Some("sec"), "still held for the real page");
    assert!(app.pending_fragment.is_none());
    let generation = app.nav_generation;
    let page = FetchedPage {
        html: b"<html><body><div style=\"height:900px\">x</div><p id=\"sec\">target</p></body></html>".to_vec(),
        extra_css: String::new(),
        sheets: Vec::new(),
        scripts: Vec::new(),
        url: Url::parse("http://127.0.0.1:9/p").unwrap(),
    };
    app.handle_nav_result(generation, Ok(page));
    assert_eq!(app.pending_fragment.as_deref(), Some("sec"));
    qa6_deliver_layout(&mut app, false);
    assert!(app.pending_fragment.is_none());
    assert!(app.scroll_offset >= 900.0, "scrolled to target, got {}", app.scroll_offset);
    assert!(app.interaction.target.is_some());

    // An existing target on an incomplete layout is applied but kept, so
    // the scroll follows content that is still growing above it.
    app.pending_fragment = Some("sec".into());
    app.scroll_offset = 0.0;
    qa6_deliver_layout(&mut app, true);
    assert_eq!(app.pending_fragment.as_deref(), Some("sec"), "kept while incomplete");
    assert!(app.scroll_offset >= 900.0);
    qa6_deliver_layout(&mut app, false);
    assert!(app.pending_fragment.is_none());

    // Target absent from an incomplete layout: keep waiting; a complete
    // layout without it gives up.
    app.pending_fragment = Some("late".into());
    app.scroll_offset = 0.0;
    qa6_deliver_layout(&mut app, true);
    assert_eq!(app.pending_fragment.as_deref(), Some("late"), "incomplete layout keeps waiting");
    qa6_deliver_layout(&mut app, false);
    assert!(app.pending_fragment.is_none(), "complete layout without the id gives up");

    // Target that appears only in a later layout generation.
    app.pending_fragment = Some("late".into());
    qa6_deliver_layout(&mut app, true);
    assert_eq!(app.pending_fragment.as_deref(), Some("late"));
    let doc = parse_html("<html><body><div style=\"height:700px\">x</div><p id=\"late\">t</p></body></html>");
    app.replace_document(doc, String::new());
    app.pending_fragment = Some("late".into());
    qa6_deliver_layout(&mut app, false);
    assert!(app.pending_fragment.is_none());
    assert!(app.scroll_offset >= 700.0, "got {}", app.scroll_offset);
}

/// Every command the command bar accepts (`COMMAND_NAMES`, also what Tab
/// completes) must be documented on the `:help` page. Fails when a new
/// command is added without a row in `src/help.html`.
#[test]
fn help_page_documents_every_command() {
    for name in COMMAND_NAMES {
        let needle = format!(":{name}");
        assert!(
            HELP_PAGE_HTML.contains(&needle),
            "`:{name}` is a command-bar command but is not documented in src/help.html"
        );
    }
}

// ---------- QJ6: element scrolling ----------

const QJ6_DOC: &str = "<html><head><style>body{margin:0} #box{height:100px;overflow:auto;position:relative;border:0}\
    .row{height:40px} #plain{height:100px;overflow:hidden}</style></head><body>\
    <div id=box><div class=row id=r0></div><div class=row id=r1></div><div class=row id=r2></div><div class=row id=r3></div><div class=row id=r4></div></div>\
    <div id=plain><div style='height:300px'></div></div></body></html>";

fn qj6_log(code: &str) -> Vec<String> {
    run_in_html(QJ6_DOC, &format!("var box=document.getElementById('box'), r4=document.getElementById('r4'); {code}"))
}

#[test]
fn qj6_scroll_top_set_clamps_and_reads_back() {
    // content 200 px, client 100 px: max scrollTop 100
    let log = qj6_log(
        "console.log(box.scrollHeight, box.clientHeight, box.scrollTop);\
         box.scrollTop = 50; console.log(box.scrollTop);\
         box.scrollTop = 9999; console.log(box.scrollTop);\
         box.scrollTop = -5; console.log(box.scrollTop);\
         var p=document.getElementById('plain'); p.scrollTop = 20; console.log('hidden', p.scrollTop);",
    );
    assert_eq!(log, ["log:200 100 0", "log:50", "log:100", "log:0", "log:hidden 0"], "{log:?}");
}

#[test]
fn qj6_scroll_methods_move_children_rects() {
    let log = qj6_log(
        "var y0=r4.getBoundingClientRect().top;\
         box.scrollBy(0, 30); console.log(box.scrollTop, y0 - r4.getBoundingClientRect().top);\
         box.scrollTo(0, 70); console.log(box.scrollTop, y0 - r4.getBoundingClientRect().top);\
         box.scroll({top: 10}); console.log(box.scrollTop);\
         box.scrollBy({top: 500}); console.log(box.scrollTop);\
         box.scrollTop = 0; console.log(y0 - r4.getBoundingClientRect().top);",
    );
    assert_eq!(log, ["log:30 30", "log:70 70", "log:10", "log:100", "log:0"], "{log:?}");
}

#[test]
fn qj6_scroll_event_fires_on_change_only() {
    let doc = parse_html(QJ6_DOC);
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let clock = Rc::new(std::cell::Cell::new(0.0));
    let engine = crate::js::start_scripts_clocked(
        &doc,
        &[src("var box=document.getElementById('box'); var n=0; box.addEventListener('scroll', function(){ n++; console.log('scroll', box.scrollTop); });\
               box.scrollTop = 40; box.scrollTop = 40; box.scrollBy(0, 0);")],
        Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))),
        Some(clock.clone()),
    )
    .unwrap();
    for _ in 0..100 {
        let Some(due) = engine.next_due_ms() else { break };
        clock.set(clock.get().max(due));
        engine.run_due();
    }
    assert_eq!(*out.borrow(), ["log:scroll 40"]);
}

#[test]
fn qj6_scroll_into_view_scrolls_ancestor_then_window() {
    let doc = "<html><head><style>body{margin:0} .s{height:2000px} #box{height:100px;overflow:auto;position:relative}\
        .row{height:40px}</style></head><body><div class=s></div>\
        <div id=box><div class=row></div><div class=row></div><div class=row></div><div class=row></div><div class=row id=t></div></div>\
        <div class=s></div></body></html>";
    let log = run_in_html(doc, "var box=document.getElementById('box'), t=document.getElementById('t');\
        t.scrollIntoView({block:'start'}); console.log(box.scrollTop, scrollY);\
        console.log(t.getBoundingClientRect().top);");
    // the row starts at 160 inside a box that can scroll 100: clamped; the
    // window then brings the box's scrolled row to the top: 2000 + 160 - 100.
    assert_eq!(log, ["log:100 2060", "log:0"], "{log:?}");
}

#[test]
fn qj6_app_applies_element_scroll_request() {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = match EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build() {
        Ok(l) => l,
        Err(_) => return, // no display available (headless CI)
    };
    let doc = parse_html(QJ6_DOC);
    let mut app = App::new_static(doc.clone(), event_loop.create_proxy(), None);
    app.js = run_page_scripts(&doc, &[src("document.getElementById('box').scrollBy(0, 50);")], None);
    app.js_sync();
    assert_eq!(app.node_scroll.values().copied().collect::<Vec<_>>(), [50.0]);
}

// ---------- QJ12: KeyboardEvent code / keyCode / which / charCode / location ----------

#[test]
fn qj12_key_codes_table() {
    use winit::keyboard::KeyCode as KC;
    let ch = |s: &str| Key::Character(s.into());
    // (logical, physical, code, keyCode, location)
    let rows: Vec<(Key, Option<KC>, &str, u32, u32)> = vec![
        (Key::Named(NamedKey::Enter), Some(KC::Enter), "Enter", 13, 0),
        (Key::Named(NamedKey::Escape), Some(KC::Escape), "Escape", 27, 0),
        (Key::Named(NamedKey::Tab), Some(KC::Tab), "Tab", 9, 0),
        (Key::Named(NamedKey::Backspace), Some(KC::Backspace), "Backspace", 8, 0),
        (Key::Named(NamedKey::Space), Some(KC::Space), "Space", 32, 0),
        (Key::Named(NamedKey::Delete), Some(KC::Delete), "Delete", 46, 0),
        (Key::Named(NamedKey::ArrowLeft), Some(KC::ArrowLeft), "ArrowLeft", 37, 0),
        (Key::Named(NamedKey::ArrowUp), Some(KC::ArrowUp), "ArrowUp", 38, 0),
        (Key::Named(NamedKey::ArrowRight), Some(KC::ArrowRight), "ArrowRight", 39, 0),
        (Key::Named(NamedKey::ArrowDown), Some(KC::ArrowDown), "ArrowDown", 40, 0),
        (Key::Named(NamedKey::Home), Some(KC::Home), "Home", 36, 0),
        (Key::Named(NamedKey::End), Some(KC::End), "End", 35, 0),
        (Key::Named(NamedKey::PageUp), Some(KC::PageUp), "PageUp", 33, 0),
        (Key::Named(NamedKey::PageDown), Some(KC::PageDown), "PageDown", 34, 0),
        (Key::Named(NamedKey::F1), Some(KC::F1), "F1", 112, 0),
        (Key::Named(NamedKey::F12), Some(KC::F12), "F12", 123, 0),
        (ch("a"), Some(KC::KeyA), "KeyA", 65, 0),
        (ch("A"), Some(KC::KeyA), "KeyA", 65, 0),
        (ch("z"), Some(KC::KeyZ), "KeyZ", 90, 0),
        (ch("1"), Some(KC::Digit1), "Digit1", 49, 0),
        (ch("!"), Some(KC::Digit1), "Digit1", 49, 0),
        (ch("0"), Some(KC::Digit0), "Digit0", 48, 0),
        (ch(";"), Some(KC::Semicolon), "Semicolon", 186, 0),
        (ch("/"), Some(KC::Slash), "Slash", 191, 0),
        (ch("]"), Some(KC::BracketRight), "BracketRight", 221, 0),
        (Key::Named(NamedKey::Shift), Some(KC::ShiftLeft), "ShiftLeft", 16, 1),
        (Key::Named(NamedKey::Shift), Some(KC::ShiftRight), "ShiftRight", 16, 2),
        (Key::Named(NamedKey::Control), Some(KC::ControlLeft), "ControlLeft", 17, 1),
        (Key::Named(NamedKey::Alt), Some(KC::AltRight), "AltRight", 18, 2),
        (ch("5"), Some(KC::Numpad5), "Numpad5", 101, 3),
        (Key::Named(NamedKey::Enter), Some(KC::NumpadEnter), "NumpadEnter", 13, 3),
        // No physical key (text-control path): derived from the logical key.
        (ch("q"), None, "KeyQ", 81, 0),
        (ch("7"), None, "Digit7", 55, 0),
        (Key::Named(NamedKey::Enter), None, "Enter", 13, 0),
        (Key::Named(NamedKey::Space), None, "Space", 32, 0),
        (ch(":"), None, "", 0, 0),
    ];
    for (logical, phys, code, kcode, loc) in rows {
        let got = key_codes(&logical, phys.map(PhysicalKey::Code), false);
        assert_eq!(
            got,
            KeyCodes { code: code.to_string(), key_code: kcode, char_code: 0, location: loc },
            "{logical:?} {phys:?}"
        );
    }
    // keypress: charCode = code point, keyCode the same.
    let kp = key_codes(&ch("a"), Some(PhysicalKey::Code(KC::KeyA)), true);
    assert_eq!((kp.char_code, kp.key_code, kp.code.as_str()), (97, 97, "KeyA"));
    let kp = key_codes(&ch("A"), Some(PhysicalKey::Code(KC::KeyA)), true);
    assert_eq!((kp.char_code, kp.key_code), (65, 65));
    let kp = key_codes(&Key::Named(NamedKey::Enter), None, true);
    assert_eq!((kp.char_code, kp.key_code), (13, 13));
}

#[test]
fn qj12_page_sees_codes_and_can_still_cancel() {
    use winit::keyboard::KeyCode as KC;
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = match EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build() {
        Ok(l) => l,
        Err(_) => return,
    };
    clear_text_state();
    let doc = parse_html("<html><body><input id=i></body></html>");
    let mut app = App::new_static(doc.clone(), event_loop.create_proxy(), None);
    app.js = run_page_scripts(
        &doc,
        &[src(
            "window.T=[]; var f=function(e){T.push([e.type,e.key,e.code,e.keyCode,e.which,e.charCode,e.location,e.shiftKey,e.repeat].join(','));\
             if(e.key==='x')e.preventDefault()};\
             document.addEventListener('keydown',f);document.addEventListener('keypress',f);document.addEventListener('keyup',f);",
        )],
        None,
    );
    let i = q30c_by(&doc, "i");
    app.set_focus_element(Some(&i), false);
    let enter = Key::Named(NamedKey::Enter);
    assert!(app.js_key_codes("keydown", &enter, Some(PhysicalKey::Code(KC::Enter)), false));
    assert!(app.js_key_for("keypress", &enter, false));
    assert!(app.js_key_codes("keyup", &enter, Some(PhysicalKey::Code(KC::Enter)), false));
    assert!(app.js_key_codes("keydown", &Key::Named(NamedKey::Escape), Some(PhysicalKey::Code(KC::Escape)), true));
    assert!(app.js_key_codes("keydown", &Key::Named(NamedKey::ArrowLeft), Some(PhysicalKey::Code(KC::ArrowLeft)), false));
    assert!(app.js_key_for("keypress", &Key::Character("a".into()), false));
    // preventDefault still cancels any key.
    assert!(!app.js_key_codes("keydown", &Key::Character("x".into()), Some(PhysicalKey::Code(KC::KeyX)), false));
    let got: String = app.js.as_ref().unwrap().eval_value("t", "T.join('|')").expect("log");
    assert_eq!(
        got,
        "keydown,Enter,Enter,13,13,0,0,false,false|keypress,Enter,Enter,13,13,13,0,false,false|keyup,Enter,Enter,13,13,0,0,false,false|\
         keydown,Escape,Escape,27,27,0,0,false,true|keydown,ArrowLeft,ArrowLeft,37,37,0,0,false,false|\
         keypress,a,KeyA,97,97,97,0,false,false|keydown,x,KeyX,88,88,0,0,false,false"
    );
    clear_text_state();
}

#[test]
fn qj12_keyboardevent_ctor_fields_and_modifiers() {
    let doc = parse_html("<html><body></body></html>");
    let js = run_page_scripts(
        &doc,
        &[src(
            "var e=new KeyboardEvent('keydown',{key:'a',code:'KeyA',keyCode:65,charCode:0,location:2,shiftKey:true,ctrlKey:true,altKey:true,metaKey:true});\
             var d=new KeyboardEvent('keydown',{key:'b'});\
             window.R=[e.key,e.code,e.keyCode,e.which,e.charCode,e.location,e.shiftKey,e.ctrlKey,e.altKey,e.metaKey,d.keyCode,d.which,d.location].join(',');",
        )],
        None,
    );
    let got: String = js.as_ref().unwrap().eval_value("t", "R").expect("r");
    assert_eq!(got, "a,KeyA,65,65,0,2,true,true,true,true,0,0,0");
}

#[test]
fn qy51_document_special_no_walk() {
    // Normal page, script-inserted element, body replacement, removal, re-insertion.
    let log = run_in_html(
        "<html><head><title>t</title></head><body><p id=a>x</p></body></html>",
        "var h=document.documentElement;\
         console.log(h.tagName, document.head.tagName, document.body.tagName, document.body===h.lastElementChild);\
         var d=document.createElement('div'); document.body.appendChild(d);\
         console.log(document.body===d.parentNode);\
         var nb=document.createElement('body'); nb.id='nb'; h.replaceChild(nb, document.body);\
         console.log(document.body.id, document.body===nb);\
         h.removeChild(nb); console.log(document.body===null);\
         h.appendChild(nb); console.log(document.body===nb);\
         h.removeChild(document.head); console.log(document.head===null);\
         h.insertBefore(document.createElement('head'), nb); console.log(document.head.tagName);\
         h.parentNode.removeChild(h); console.log(document.documentElement===null, document.body===null, document.head===null);",
    );
    assert_eq!(log[0], "log:HTML HEAD BODY true", "{log:?}");
    assert_eq!(log[1], "log:true", "{log:?}");
    assert_eq!(log[2], "log:nb true", "{log:?}");
    assert_eq!(log[3], "log:true", "{log:?}");
    assert_eq!(log[4], "log:true", "{log:?}");
    assert_eq!(log[5], "log:true", "{log:?}");
    assert_eq!(log[6], "log:HEAD", "{log:?}");
    assert_eq!(log[7], "log:true true true", "{log:?}");
}

#[test]
fn qy51_document_special_duplicates_and_frameset() {
    // First matching child of html wins; a body inside head is not the body.
    let log = run_in_html(
        "<html><head id=h1></head><body id=b1></body></html>",
        "var h=document.documentElement;\
         var h2=document.createElement('head'); h2.id='h2'; h.appendChild(h2);\
         var b2=document.createElement('body'); b2.id='b2'; h.appendChild(b2);\
         console.log(document.head.id, document.body.id);\
         var fs=document.createElement('frameset'); fs.id='fs';\
         h.replaceChild(fs, document.body); h.removeChild(b2);\
         console.log(document.body.id);",
    );
    assert_eq!(log[0], "log:h1 b1", "{log:?}");
    assert_eq!(log[1], "log:fs", "{log:?}");
}

// ---------- QYSCR1: overflow viewport propagation (YouTube scrolling) ----------

/// Every scroll region in `laid`, in-flow and positioned.
fn qyscr1_regions(laid: &Laid, out: &mut Vec<ScrollInfo>) {
    if let Some(i) = laid.scroll {
        out.push(i);
    }
    let kids: &[Laid] = match &laid.content {
        LaidContent::Blocks(c) | LaidContent::TableRow(c) => c,
        _ => &[],
    };
    kids.iter().chain(laid.positioned.iter()).for_each(|c| qyscr1_regions(c, out));
}

/// YouTube's shape: `body{overflow-y:scroll}` and the whole page in an
/// absolutely positioned app box, so `<body>` itself is 0 px tall.
const QYSCR1_YT: &str = "<html><head><style>body{margin:0;padding:0;overflow-y:scroll}\
    #app{position:absolute;top:0;left:0;right:0} .row{height:100px}</style></head><body>\
    <div id=app><div class=row></div><div class=row></div><div class=row></div><div class=row></div><div class=row></div>\
    <div class=row></div><div class=row></div><div class=row></div><div class=row></div><div class=row></div>\
    <div class=row></div><div class=row></div><div class=row></div><div class=row></div><div class=row></div></div></body></html>";

#[test]
fn qyscr1_body_overflow_scroll_with_abs_content_scrolls_the_viewport() {
    let laid = measure_document_fixture(QYSCR1_YT, 800.0, 600.0);
    // The page is as tall as the absolutely positioned app (15 rows), so the
    // top-level scroll (`redraw` clamps to `laid.h`) can reach its end.
    assert!(laid.h >= 1500.0, "document height {} must include the absolute app box", laid.h);
    // <body>'s `overflow` went to the viewport: no 0 px body scroll region.
    let mut regions = Vec::new();
    qyscr1_regions(&laid, &mut regions);
    assert!(regions.is_empty(), "no element scroll region expected, got {}", regions.len());
    // So the wheel finds no region anywhere and scrolls the page.
    for y in [0.0, 50.0, 500.0, 1200.0] {
        assert!(hit_test_scroll_region(&laid, 100.0, y, &HashMap::new(), 0.0, 600.0).is_none(), "y={y}");
    }
}

#[test]
fn qyscr1_body_overflow_scroll_with_in_flow_content_is_no_wheel_trap() {
    let rows = "<div style='height:100px'></div>".repeat(15);
    let html = format!("<html><head><style>body{{margin:0;overflow-y:scroll}}</style></head><body>{rows}</body></html>");
    let laid = measure_document_fixture(&html, 800.0, 600.0);
    assert!(laid.h >= 1500.0, "document height {}", laid.h);
    let mut regions = Vec::new();
    qyscr1_regions(&laid, &mut regions);
    assert!(regions.is_empty(), "<body> must not be a scroll region");
    assert!(hit_test_scroll_region(&laid, 100.0, 300.0, &HashMap::new(), 0.0, 600.0).is_none());
}

#[test]
fn qyscr1_html_overflow_propagates_first_and_body_keeps_its_own() {
    // <html>'s non-visible overflow goes to the viewport; <body>'s then stays
    // on <body> (CSS Overflow 3), so a short overflow:auto body still scrolls.
    let rows = "<div style='height:100px'></div>".repeat(5);
    let html = format!(
        "<html style='overflow:hidden'><head><style>body{{margin:0;height:200px;overflow:auto}}</style></head><body>{rows}</body></html>"
    );
    let laid = measure_document_fixture(&html, 800.0, 600.0);
    let mut regions = Vec::new();
    qyscr1_regions(&laid, &mut regions);
    assert_eq!(regions.len(), 1, "only <body> is a scroll region");
    assert!((regions[0].client_h - 200.0).abs() < 0.5 && regions[0].max_scroll() > 250.0);
    assert!(hit_test_scroll_region(&laid, 100.0, 50.0, &HashMap::new(), 0.0, 600.0).is_some());
}

#[test]
fn qyscr1_root_paint_containment_keeps_body_overflow_on_body() {
    // WPT css-overflow/overflow-body-propagation-011: `contain: paint` on
    // <html> stops the propagation, so <body> stays a scroll container.
    let html = "<html style='contain:paint'><head><style>body{margin:0;height:100px;overflow:scroll}</style></head>\
        <body><div style='height:300px'></div></body></html>";
    let laid = measure_document_fixture(html, 800.0, 600.0);
    let mut regions = Vec::new();
    qyscr1_regions(&laid, &mut regions);
    assert_eq!(regions.len(), 1, "<body> keeps its own scroll region");
    let doc = parse_html("<html><head><style>html{contain: layout style} p{contain:size}</style></head><body><p>x</p></body></html>");
    let tree = build_layout_tree(&doc, 800.0);
    let BoxContent::Blocks(top) = &tree.content else { panic!("root blocks") };
    assert!(top[0].style.contain_lp, "`layout` counts");
}

#[test]
fn qyscr1_fixed_boxes_do_not_add_page_height_but_absolute_ones_do() {
    let fixed = "<html><head><style>body{margin:0} #f{position:fixed;top:0;left:0;width:10px;height:5000px}</style></head>\
        <body><div style='height:100px'></div><div id=f></div></body></html>";
    let laid = measure_document_fixture(fixed, 800.0, 600.0);
    assert!(laid.h < 1000.0, "a fixed box never scrolls, so it adds nothing (h = {})", laid.h);
    let abs = "<html><head><style>body{margin:0} #a{position:absolute;top:2000px;left:0;width:10px;height:50px}</style></head>\
        <body><div style='height:100px'></div><div id=a></div></body></html>";
    let laid = measure_document_fixture(abs, 800.0, 600.0);
    assert!(laid.h >= 2050.0, "an absolute box at top:2000px extends the page (h = {})", laid.h);
}

#[test]
fn qyscr1_a_region_with_nothing_to_scroll_is_no_wheel_target() {
    // `overflow: scroll` on a box whose content fits: the wheel goes past it.
    let html = "<html><body style='margin:0'><div style='overflow:scroll;height:100px'><div style='height:50px'></div></div>\
        <div style='height:2000px'></div></body></html>";
    let laid = measure_document_fixture(html, 800.0, 600.0);
    let fits = hit_test_scroll_region(&laid, 10.0, 10.0, &HashMap::new(), 0.0, 600.0).expect("the overflow:scroll box");
    assert!(!region_takes_wheel(&fits, &HashMap::new(), -40.0), "nothing to scroll: the page takes the tick");
    assert!(!region_takes_wheel(&fits, &HashMap::new(), 40.0));
    // A region that can scroll takes ticks until its end, then chains.
    let tall = ScrollInfo { id: 7, content_h: 300.0, client_h: 100.0, node: 0 };
    let mut offs = HashMap::new();
    assert!(region_takes_wheel(&tall, &offs, -40.0), "down from the top");
    assert!(!region_takes_wheel(&tall, &offs, 40.0), "up at the top: chains to the page");
    offs.insert(7, 200.0);
    assert!(!region_takes_wheel(&tall, &offs, -40.0), "down at the end: chains to the page");
    assert!(region_takes_wheel(&tall, &offs, 40.0));
}

#[test]
fn qyscr1_space_page_and_arrow_keys_scroll() {
    let n = VimMode::Normal;
    let m = ModifiersState::empty();
    assert_eq!(resolve_binding(&Key::Named(NamedKey::Space), m, n), Some(Action::PageDown));
    assert_eq!(resolve_binding(&Key::Named(NamedKey::Space), ModifiersState::SHIFT, n), Some(Action::PageUp));
    assert_eq!(resolve_binding(&Key::Named(NamedKey::PageDown), m, n), Some(Action::PageDown));
    assert_eq!(resolve_binding(&Key::Named(NamedKey::PageUp), m, n), Some(Action::PageUp));
    assert_eq!(resolve_binding(&Key::Named(NamedKey::ArrowDown), m, n), Some(Action::ScrollDown));
    assert_eq!(resolve_binding(&Key::Named(NamedKey::ArrowUp), m, n), Some(Action::ScrollUp));
    assert_eq!(resolve_binding(&Key::Named(NamedKey::Space), m, VimMode::Command), None);
    assert!(!action_ignores_repeat(Action::PageDown));
    assert_eq!(page_step(600.0), 560.0);
    assert_eq!(page_step(10.0), 40.0);
}

#[test]
fn qyscr1_script_sees_body_as_no_scroll_container_and_scrolls_the_window() {
    let log = run_in_html(
        QYSCR1_YT,
        "var b=document.body, t=document.querySelectorAll('.row')[12];\
         console.log(getComputedStyle(b).overflowY);\
         b.scrollTop = 50; console.log(b.scrollTop);\
         t.scrollIntoView({block:'start'}); console.log(scrollY > 0 && scrollY === Math.min(1200, 1500 - innerHeight));",
    );
    // The computed value stays `scroll`; the body is not scrollable, so
    // scrollIntoView moves the window towards the row (12 * 100 px), clamped to
    // the 1500 px page (the absolute app box) less the viewport.
    assert_eq!(log, ["log:scroll", "log:0", "log:true"], "{log:?}");
}
