#[allow(unused_imports)]
use super::*;

// ---------- QF2: Blob, File, FileReader, blob: URL store ----------

#[test]
fn qf2_blob_size_type_slice() {
    let log = run_with_timers(
        "var b = new Blob(['héllo ', new Uint8Array([1, 2, 3]), new ArrayBuffer(2), new DataView(new ArrayBuffer(4))], {type: 'Text/Plain'});\
         console.log(b.size, b.type);\
         console.log(new Blob([], {type: 'a\\u00e9'}).type === '', new Blob().size);\
         var s = b.slice(-3); console.log(s.size, s.type === '');\
         var t = new Blob(['abcdef']).slice(1, -1, 'X/Y'); console.log(t.size, t.type);\
         console.log(new Blob(['abc']).slice(5).size, new Blob(['abc']).slice(2, 1).size);\
         console.log(new Blob(['a\\r\\nb\\rc'], {endings: 'native'}).size, new Blob(['a\\r\\nb']).size);\
         console.log(Object.prototype.toString.call(b), b instanceof Blob);",
    );
    assert_eq!(
        log,
        ["log:16 text/plain", "log:true 0", "log:3 true", "log:4 x/y", "log:0 0", "log:5 4", "log:[object Blob] true"],
        "{log:?}"
    );
}

#[test]
fn qf2_blob_text_arraybuffer_and_nested() {
    let log = run_with_timers(
        "var inner = new Blob(['é', '€']);\
         var outer = new Blob(['x', inner, new Uint8Array([0x41])]);\
         outer.text().then(function (t) { console.log('text', t, outer.size); });\
         outer.arrayBuffer().then(function (ab) { console.log('ab', Array.prototype.join.call(new Uint8Array(ab), ',')); });\
         outer.bytes().then(function (u) { console.log('bytes', u.length); });\
         new Blob([new Uint8Array([0xEF, 0xBB, 0xBF, 0x68, 0xFF])]).text().then(function (t) { console.log('bom', t.length, t.charCodeAt(1)); });",
    );
    assert_eq!(
        log,
        ["log:text xé€A 7", "log:ab 120,195,169,226,130,172,65", "log:bytes 7", "log:bom 2 65533"],
        "{log:?}"
    );
}

#[test]
fn qf2_file_properties() {
    let log = run_with_timers(
        "var f = new File(['hi'], 'a.txt', {type: 'text/plain', lastModified: 1234});\
         console.log(f.name, f.size, f.type, f.lastModified, JSON.stringify(f.webkitRelativePath), f instanceof Blob, f instanceof File);\
         console.log(typeof new File([], 'x').lastModified, f.slice(1).size);\
         try { new File([]); } catch (e) { console.log(e.name); }",
    );
    assert_eq!(log, ["log:a.txt 2 text/plain 1234 \"\" true true", "log:number 1", "log:TypeError"], "{log:?}");
}

#[test]
fn qf2_filereader_events_and_results() {
    let log = run_with_timers(
        "var b = new Blob(['hé'], {type: 'text/plain'});\
         var r = new FileReader(); var ev = [];\
         console.log(r.readyState, r.result);\
         ['loadstart', 'progress', 'load', 'loadend'].forEach(function (n) { r.addEventListener(n, function (e) { ev.push(n + ':' + r.readyState + ':' + e.loaded); }); });\
         r.onload = function () { var res = r.result; setTimeout(function () { console.log('onload', res, ev.join(' ')); }, 1); };\
         r.readAsText(b); console.log(r.readyState, ev.length);\
         try { r.readAsText(b); } catch (e) { console.log(e.name); }\
         var r2 = new FileReader(); r2.onloadend = function () { console.log(r2.result); }; r2.readAsDataURL(b);\
         var r3 = new FileReader(); r3.onload = function () { console.log(r3.result.byteLength, r3.result instanceof ArrayBuffer); }; r3.readAsArrayBuffer(b);\
         var r4 = new FileReader(); r4.onload = function () { console.log(r4.result.length); }; r4.readAsBinaryString(b);\
         var r5 = new FileReader(); var e5 = []; r5.onabort = function () { e5.push('abort'); }; r5.onloadend = function () { e5.push('end'); }; r5.onload = function () { e5.push('load'); };\
         r5.readAsText(b); r5.abort(); console.log(r5.readyState, r5.result);\
         setTimeout(function () { console.log(e5.join(',')); }, 10);",
    );
    assert_eq!(
        log,
        [
            "log:0 null",
            "log:1 0",
            "log:InvalidStateError",
            "log:2 null",
            "log:data:text/plain;base64,aMOp",
            "log:3 true",
            "log:3",
            "log:onload h\u{e9} loadstart:1:0 progress:1:3 load:2:3 loadend:2:3",
            "log:abort,end",
        ],
        "{log:?}"
    );
}

#[test]
fn qf2_object_url_registered_revoked_and_lookup() {
    let log = run_with_timers(
        "var b = new Blob(['zz']); var u = URL.createObjectURL(b); var v = URL.createObjectURL(b);\
         console.log(/^blob:.*\\/[0-9a-f-]{36}$/.test(u) || u, u !== v);\
         console.log(__blob_lookup(u) === b, __blob_lookup(u + '#frag') === b, __blob_lookup('blob:nope'));\
         URL.revokeObjectURL(u); console.log(__blob_lookup(u), __blob_lookup(v) === b);\
         try { URL.createObjectURL({}); } catch (e) { console.log(e.name); }",
    );
    assert_eq!(log, ["log:true true", "log:true true undefined", "log:undefined true", "log:TypeError"], "{log:?}");
}

// ---------- QF4: ReadableStream, Headers, Request, Response ----------

#[test]
fn qf4_headers_normalise_combine_and_iterate_sorted() {
    let log = run_with_timers(
        "var h = new Headers({'X-B': ' two ', 'a-a': '1'});\
         h.append('x-b', '3'); h.append('Set-Cookie', 'a=1'); h.append('set-cookie', 'b=2');\
         console.log(JSON.stringify(Array.from(h)));\
         console.log(h.get('X-B'), h.get('nope'), h.has('A-A'), JSON.stringify(h.getSetCookie()));\
         h.set('X-B', 'only'); h.delete('a-a');\
         console.log(JSON.stringify(Array.from(h.keys())), JSON.stringify(Array.from(h.values())));\
         var seen = []; h.forEach(function (v, k, o) { seen.push(k + '=' + v + (o === h)); }); console.log(seen.join(';'));\
         console.log(JSON.stringify(Array.from(new Headers([['b', '1'], ['A', '2']]))), JSON.stringify(Array.from(new Headers(h))));\
         var bad = [function () { new Headers({'bad name': 'x'}); }, function () { new Headers().append('a', 'x\\ny'); },\
           function () { new Headers([['a']]); }, function () { new Headers().append('a', '\\u0100'); }, function () { new Headers(5); }];\
         console.log(bad.map(function (f) { try { f(); return 'ok'; } catch (e) { return e.name; } }).join());\
         console.log(Object.prototype.toString.call(h), typeof h[Symbol.iterator], new Headers().get('x'));",
    );
    assert_eq!(
        log,
        [
            "log:[[\"a-a\",\"1\"],[\"set-cookie\",\"a=1\"],[\"set-cookie\",\"b=2\"],[\"x-b\",\"two, 3\"]]",
            "log:two, 3 null true [\"a=1\",\"b=2\"]",
            "log:[\"set-cookie\",\"set-cookie\",\"x-b\"] [\"a=1\",\"b=2\",\"only\"]",
            "log:set-cookie=a=1true;set-cookie=b=2true;x-b=onlytrue",
            "log:[[\"a\",\"2\"],[\"b\",\"1\"]] [[\"set-cookie\",\"a=1\"],[\"set-cookie\",\"b=2\"],[\"x-b\",\"only\"]]",
            "log:TypeError,TypeError,TypeError,TypeError,TypeError",
            "log:[object Headers] function null",
        ],
        "{log:?}"
    );
}

#[test]
fn qf4_headers_guards_forbidden_and_immutable() {
    let log = run_with_timers(
        "var r = new Request('http://a.test/', {headers: {Cookie: 'x=1', Host: 'h', 'Sec-Fetch-Mode': 'x', 'Proxy-Foo': '1', Origin: 'o', 'X-Ok': '1', 'Content-Length': '3'}});\
         console.log(JSON.stringify(Array.from(r.headers)));\
         r.headers.set('Referer', 'x'); r.headers.append('Via', 'x'); console.log(r.headers.has('referer'), r.headers.has('via'));\
         var s = new Response('x', {headers: {'Set-Cookie': 'a=1', 'X-Ok': '2'}});\
         console.log(JSON.stringify(Array.from(s.headers)), s.headers.getSetCookie().length);\
         var e = Response.error();\
         try { e.headers.set('a', 'b'); } catch (x) { console.log(x.name); }\
         var net = __make_response(200, 'OK', [['Set-Cookie', 'a=1'], ['Content-Type', 'text/x'], ['X', '1'], ['x', '2']], new Uint8Array([104, 105]), 'http://a.test/p', true, 'basic');\
         console.log(JSON.stringify(Array.from(net.headers)), net.headers.get('set-cookie'), net.url, net.redirected, net.type);\
         try { net.headers.append('a', 'b'); } catch (x) { console.log(x.name); }\
         var op = __make_response(200, 'OK', [['a', 'b']], new Uint8Array([1]), 'http://a.test/', false, 'opaque');\
         console.log(op.status, op.url === '', op.body, Array.from(op.headers).length);\
         net.text().then(function (t) { console.log(t, net.bodyUsed); });",
    );
    assert_eq!(
        log,
        [
            "log:[[\"x-ok\",\"1\"]]",
            "log:false false",
            "log:[[\"content-type\",\"text/plain;charset=UTF-8\"],[\"x-ok\",\"2\"]] 0",
            "log:TypeError",
            "log:[[\"content-type\",\"text/x\"],[\"x\",\"1, 2\"]] null http://a.test/p true basic",
            "log:TypeError",
            "log:0 true null 0",
            "log:hi true",
        ],
        "{log:?}"
    );
}

#[test]
fn qf4_readablestream_reader_read_done_cancel() {
    let log = run_with_timers(
        "var ctl, cancelled = [], rs = new ReadableStream({start: function (c) { ctl = c; c.enqueue('a'); }, cancel: function (r) { cancelled.push(r); }});\
         console.log(rs.locked, ctl.desiredSize, typeof rs.tee, typeof rs.pipeTo, typeof TransformStream);\
         var rd = rs.getReader(); console.log(rs.locked);\
         try { rs.getReader(); } catch (e) { console.log(e.name); }\
         rd.read().then(function (r) { console.log(JSON.stringify(r)); ctl.enqueue('b'); ctl.close(); return rd.read(); })\
           .then(function (r) { console.log(JSON.stringify(r)); return rd.read(); })\
           .then(function (r) { console.log(r.done, r.value); return rd.closed; })\
           .then(function () { rd.releaseLock(); console.log(rs.locked); return rd.read().catch(function (e) { console.log(e.name); }); })\
           .then(function () {\
              var s2 = new ReadableStream({pull: function (c) { c.enqueue(1); }, cancel: function (r) { cancelled.push(r); }}, {highWaterMark: 0});\
              var r2 = s2.getReader();\
              return r2.read().then(function (x) { console.log('pulled', x.value); return r2.cancel('bye'); }).then(function () { console.log(cancelled.join()); return r2.read(); });\
           }).then(function (x) { console.log('after cancel', x.done);\
              var s3 = new ReadableStream({start: function (c) { c.error(new Error('boom')); }});\
              return s3.getReader().read().catch(function (e) { console.log(e.message); });\
           }).then(function () {\
              var s4 = new ReadableStream({start: function (c) { c.enqueue('x'); c.enqueue('y'); c.close(); }}), out = [];\
              return (async function () { for await (var v of s4) out.push(v); console.log(out.join(), s4.locked); })();\
           });",
    );
    assert_eq!(
        log,
        [
            "log:false 0 undefined undefined undefined",
            "log:true",
            "log:TypeError",
            "log:{\"value\":\"a\",\"done\":false}",
            "log:{\"value\":\"b\",\"done\":false}",
            "log:true undefined",
            "log:false",
            "log:TypeError",
            "log:pulled 1",
            "log:bye",
            "log:after cancel true",
            "log:boom",
            "log:x,y false",
        ],
        "{log:?}"
    );
}

#[test]
fn qf4_request_method_url_headers_and_body_rules() {
    let log = run_with_timers(
        "var r = new Request('http://a.test/p?q#f', {method: 'post', body: 'héllo', headers: {A: '1'}});\
         console.log(r.url, r.method, r.mode, r.credentials, r.cache, r.redirect, r.referrer, r.keepalive, JSON.stringify(Array.from(r.headers)));\
         console.log(new Request('http://a.test/', {method: 'patch'}).method, new Request('http://a.test/', {method: 'Options'}).method);\
         var t = ['CONNECT', 'trace', 'TRACK', 'bad method'].map(function (m) { try { new Request('http://a.test/', {method: m}); return 'ok'; } catch (e) { return e.name; } });\
         console.log(t.join());\
         var g = [function () { new Request('http://a.test/', {body: 'x'}); }, function () { new Request('http://a.test/', {method: 'HEAD', body: 'x'}); },\
           function () { new Request('http://u:p@a.test/'); }, function () { new Request('/rel'); }, function () { new Request(); },\
           function () { new Request('http://a.test/', {mode: 'navigate'}); }, function () { new Request('http://a.test/', {cache: 'zzz'}); }];\
         console.log(g.map(function (f) { try { f(); return 'ok'; } catch (e) { return e.name; } }).join());\
         var c = new Request(r, {method: 'PUT'}); console.log(c.method, c.url, r.bodyUsed, c.headers.get('a'), c.headers.get('content-type'));\
         var ab = new AbortController(), q = new Request('http://a.test/', {signal: ab.signal});\
         console.log(q.signal.aborted, q.signal === ab.signal); ab.abort(); console.log(q.signal.aborted);\
         console.log(new Request('http://a.test/', {signal: AbortSignal.abort()}).signal.aborted, Object.prototype.toString.call(q));\
         var u = new Request('http://a.test/', {method: 'POST', body: new URLSearchParams({a: '1 2'})});\
         console.log(u.headers.get('content-type'));\
         u.text().then(function (s) { console.log(s, u.bodyUsed); });\
         new Request('http://a.test/', {method: 'POST', body: new Blob(['xy'], {type: 'text/Foo'})}).text().then(function (s) { console.log(s); });\
         new Request('http://a.test/', {method: 'POST', body: new Uint8Array([104, 105])}).arrayBuffer().then(function (b) { console.log(b.byteLength); });\
         try { new Request('http://a.test/', {method: 'POST', body: {}}).text().then(function (s) { console.log(s); }); } catch (e) { console.log(e.name); }",
    );
    assert_eq!(
        log,
        [
            "log:http://a.test/p?q#f POST cors same-origin default follow about:client false [[\"a\",\"1\"],[\"content-type\",\"text/plain;charset=UTF-8\"]]",
            "log:patch OPTIONS",
            "log:TypeError,TypeError,TypeError,TypeError",
            "log:TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError",
            "log:PUT http://a.test/p?q#f true 1 text/plain;charset=UTF-8",
            "log:false false",
            "log:true",
            "log:true [object Request]",
            "log:application/x-www-form-urlencoded;charset=UTF-8",
            "log:a=1+2 true",
            "log:xy",
            "log:2",
            "log:[object Object]",
        ],
        "{log:?}"
    );
}

#[test]
fn qf4_request_relative_url_resolves_against_base_href() {
    set_page_url(Url::parse("https://page.test/dir/index.html").ok().as_ref());
    let log = run_in_html(
        "<html><head><base href=https://x.org/d/></head><body></body></html>",
        "console.log(new Request('').url, new Request('sub/x?y').url, new Request('/abs').url);\
         console.log(Response.redirect('r', 301).headers.get('location'));",
    );
    assert_eq!(log, ["log:https://x.org/d/ https://x.org/d/sub/x?y https://x.org/abs", "log:https://x.org/d/r"], "{log:?}");
    set_page_url(None);
}

#[test]
fn qf4_request_clone_and_body_used() {
    let log = run_with_timers(
        "var r = new Request('http://a.test/', {method: 'POST', body: 'abc'});\
         var c = r.clone(); console.log(r.bodyUsed, c.bodyUsed, c.method, c.url, c.headers === r.headers, c.headers.get('content-type'));\
         r.text().then(function (t) {\
           console.log(t, r.bodyUsed, c.bodyUsed);\
           return r.text().then(function () { console.log('no'); }, function (e) { console.log(e.name, /already used/.test(e.message)); });\
         }).then(function () {\
           try { r.clone(); } catch (e) { console.log('clone', e.name); }\
           try { new Request(r); } catch (e) { console.log('ctor', e.name); }\
           return c.json().catch(function (e) { console.log('json', e.name); });\
         }).then(function () {\
           var n = new Request('http://a.test/', {method: 'POST', body: 'zz'}), m = new Request(n);\
           console.log(n.bodyUsed, m.bodyUsed); return m.text();\
         }).then(function (t) { console.log(t);\
           var ab = new Request('http://a.test/', {method: 'POST', body: 'qq'}).body;\
           console.log(ab instanceof ReadableStream, new Request('http://a.test/').body);\
           return ab.getReader().read();\
         }).then(function (x) { console.log(x.value.length, x.done); });",
    );
    assert_eq!(
        log,
        [
            "log:false false POST http://a.test/ false text/plain;charset=UTF-8",
            "log:abc true false",
            "log:TypeError true",
            "log:clone TypeError",
            "log:ctor TypeError",
            "log:json SyntaxError",
            "log:true false",
            "log:zz",
            "log:true null",
            "log:2 false",
        ],
        "{log:?}"
    );
}

#[test]
fn qf4_response_status_json_text_clone_error_redirect() {
    let log = run_with_timers(
        "var r = new Response('hi', {status: 201, statusText: 'Made', headers: {X: '1'}});\
         console.log(r.status, r.statusText, r.ok, r.type, JSON.stringify(r.url), r.redirected, r.headers.get('content-type'), r.headers.get('x'));\
         var bad = [99, 600, 0, 1000].map(function (s) { try { new Response('', {status: s}); return 'ok'; } catch (e) { return e.name; } });\
         console.log(bad.join(), new Response().status, new Response(null, {status: 404}).ok, new Response().statusText === '');\
         try { new Response('x', {status: 204}); } catch (e) { console.log(e.name); }\
         try { new Response('x', {statusText: 'a\\nb'}); } catch (e) { console.log(e.name); }\
         var e = Response.error(); console.log(e.type, e.status, e.ok, e.body);\
         var d = Response.redirect('http://a.test/x', 307); console.log(d.status, d.headers.get('location'), d.type);\
         try { Response.redirect('http://a.test/', 200); } catch (x) { console.log(x.name); }\
         var j = Response.json({a: [1, 'é']}, {status: 202, headers: {'x-y': 'z'}});\
         console.log(j.status, j.headers.get('content-type'), j.headers.get('x-y'));\
         try { Response.json(undefined); } catch (x) { console.log(x.name); }\
         j.clone().text().then(function (t) { console.log(t, j.bodyUsed); return j.json(); })\
          .then(function (v) { console.log(JSON.stringify(v), j.bodyUsed); return j.text().catch(function (x) { console.log(x.name, /already used/.test(x.message)); }); })\
          .then(function () { try { j.clone(); } catch (x) { console.log('clone', x.name); }\
            return new Response('héllo', {headers: {'content-type': 'Text/HTML'}}).blob(); })\
          .then(function (b) { console.log(b.size, b.type, b instanceof Blob);\
            return new Response(new Uint8Array([1, 2, 3])).arrayBuffer(); })\
          .then(function (b) { console.log(b.byteLength); return new Response(new Blob(['xyz'], {type: 'a/b'})).bytes(); })\
          .then(function (b) { console.log(Array.from(b).join(), new Response(new Blob(['x'], {type: 'a/b'})).headers.get('content-type'));\
            return new Response('a=1&b=2').formData().catch(function (x) { console.log('fd', x.name === 'NotSupportedError' || x.name === 'TypeError'); }); })\
          .then(function () { return new Response(new ReadableStream({start: function (c) { c.enqueue(new Uint8Array([104])); c.enqueue(new Uint8Array([105])); c.close(); }})).text(); })\
          .then(function (t) { console.log('stream', t); });",
    );
    assert_eq!(
        log,
        [
            "log:201 Made true default \"\" false text/plain;charset=UTF-8 1",
            "log:RangeError,RangeError,RangeError,RangeError 200 false true",
            "log:TypeError",
            "log:TypeError",
            "log:error 0 false null",
            "log:307 http://a.test/x default",
            "log:RangeError",
            "log:202 application/json z",
            "log:TypeError",
            "log:{\"a\":[1,\"é\"]} false",
            "log:{\"a\":[1,\"é\"]} true",
            "log:TypeError true",
            "log:clone TypeError",
            "log:6 text/html true",
            "log:3",
            "log:120,121,122 a/b",
            "log:fd true",
            "log:stream hi",
        ],
        "{log:?}"
    );
}

#[test]
fn qf4_blob_stream_and_body_stream_read_marks_used() {
    let log = run_with_timers(
        "var b = new Blob(['abc']); console.log(typeof b.stream);\
         b.stream().getReader().read().then(function (x) { console.log(x.value.length, x.done); });\
         var r = new Response('abc'); var rd = r.body.getReader(); console.log(r.bodyUsed);\
         rd.read().then(function () { console.log(r.bodyUsed); return r.text().catch(function (e) { console.log(e.name); }); });",
    );
    assert_eq!(log, ["log:function", "log:false", "log:3 false", "log:true", "log:TypeError"], "{log:?}");
}

// ---------- QF3: FormData (data class; no form reading, no submission) ----------

#[test]
fn qf3_formdata_append_get_getall_has_set_delete() {
    let log = run_with_timers(
        "var f = new FormData(); f.append('a', '1'); f.append('b', 2); f.append('a', true);\
         console.log(f.get('a'), f.get('b'), f.get('zz'), f.getAll('a').join('|'), f.getAll('nope').length);\
         console.log(f.has('a'), f.has('c'));\
         f.set('a', 'x'); console.log(f.getAll('a').join('|'), f.get('b'));\
         f.set('new', 'n'); console.log(f.get('new'));\
         f.delete('a'); console.log(f.has('a'), f.getAll('a').length);\
         f.append('wh\\ud800', 'v\\udc00'); console.log(f.get('wh\\ufffd'), f.get('wh\\ufffd').length, f.get('wh\\ud800') === 'v\\ufffd');\
         try { f.append('only'); } catch (e) { console.log(e.name); }",
    );
    assert_eq!(
        log,
        [
            "log:1 2 null 1|true 0",
            "log:true false",
            "log:x 2",
            "log:n",
            "log:false 0",
            "log:v\u{fffd} 2 true",
            "log:TypeError",
        ],
        "{log:?}"
    );
}

#[test]
fn qf3_formdata_iteration_order_and_for_each() {
    let log = run_with_timers(
        "var f = new FormData(); f.append('b', '1'); f.append('a', '2'); f.append('b', '3');\
         console.log(Array.from(f.keys()).join(','), Array.from(f.values()).join(','));\
         console.log(Array.from(f).map(function (e) { return e[0] + '=' + e[1]; }).join('&'));\
         console.log(f[Symbol.iterator] === f.entries, Object.prototype.toString.call(f), Object.prototype.toString.call(f.entries()));\
         var seen = []; f.forEach(function (v, k, fd) { seen.push(k + ':' + v + ':' + (fd === f)); }, null); console.log(seen.join(' '));\
         var it = f.keys(); console.log(it.next().value, it.next().value, it.next().value, it.next().done, it.next().done);\
         try { f.forEach(1); } catch (e) { console.log(e.name); }",
    );
    assert_eq!(
        log,
        [
            "log:b,a,b 1,2,3",
            "log:b=1&a=2&b=3",
            "log:true [object FormData] [object FormData Iterator]",
            "log:b:1:true a:2:true b:3:true",
            "log:b a b true true",
            "log:TypeError",
        ],
        "{log:?}"
    );
}

#[test]
fn qf3_formdata_blob_value_becomes_file() {
    let log = run_with_timers(
        "var fd = new FormData();\
         fd.append('p', new Blob(['abc'], {type: 'text/plain'}));\
         var v = fd.get('p'); console.log(v instanceof File, v.name, v.size, v.type);\
         v.text().then(function (t) { console.log('text', t); });\
         fd.append('q', new File(['xy'], 'orig.txt', {type: 'a/b'})); console.log(fd.get('q').name);\
         fd.append('r', new File(['xy'], 'orig.txt'), 'renamed.txt'); console.log(fd.get('r').name, fd.get('r').size);\
         fd.append('s', new Blob(['z']), 'named'); console.log(fd.get('s').name, fd.get('s') instanceof File);\
         fd.append('t', 42, 'ignored'); console.log(typeof fd.get('t'), fd.get('t'));\
         fd.append('u', new Blob(['w'])); console.log(fd.getAll('u')[0].name);",
    );
    assert_eq!(
        log,
        [
            "log:true blob 3 text/plain",
            "log:orig.txt",
            "log:renamed.txt 2",
            "log:named true",
            "log:string 42",
            "log:blob",
            "log:text abc",
        ],
        "{log:?}"
    );
}

#[test]
fn qf3_formdata_form_argument_logs_and_is_empty() {
    // Q30e: FormData(form) now reads the form (an empty form gives an empty set, no log).
    let log = run_with_timers(
        "var none = new FormData();\
         var fd = new FormData(document.createElement('form'), null);\
         console.log(fd.has('x'), Array.from(fd).length, none.keys().next().done);",
    );
    assert_eq!(
        log,
        ["log:false 0 true"],
        "{log:?}"
    );
}

#[test]
fn qf3_formdata_request_and_response_bodies() {
    let log = run_with_timers(
        "var fd = new FormData(); fd.append('a', 'x\\ny\"z'); fd.append('f', new File(['hi'], 'h.txt', {type: 'text/plain'}));\
         var req = new Request('http://x.test/', {method: 'POST', body: fd}), ct = req.headers.get('content-type');\
         req.text().then(function (t) {\
           var bd = ct.split('boundary=')[1];\
           console.log(ct.indexOf('multipart/form-data; boundary=----scratchyBoundary') === 0, t.indexOf('--' + bd + '\\r\\n') === 0,\
             t.indexOf('name=\"a\"') > 0, t.indexOf('x\\r\\ny\"z') > 0,\
             t.indexOf('name=\"f\"; filename=\"h.txt\"\\r\\nContent-Type: text/plain\\r\\n\\r\\nhi\\r\\n') > 0,\
             t.endsWith('--' + bd + '--\\r\\n'), t.split('--' + bd).length === 4);\
         });\
         new Response(new URLSearchParams('a=1&a=2&b=%20')).formData().then(function (f) { console.log(f.getAll('a').join('|'), f.get('b').length, f instanceof FormData); });\
         new Response(fd).formData().catch(function (e) { console.log(e.name); });",
    );
    assert_eq!(
        log,
        [
            "log:true true true true true true true",
            "log:1|2 1 true",
            "log:TypeError",
        ],
        "{log:?}"
    );
}

#[test]
fn qf7_same_origin_request_resolves_in_order() {
    let fake = QfFake::new(vec![("/a", qf7_ok("AAA", &[])), ("/b", qf7_ok("BBB", &[]))]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.eval("t", &format!("{}{}", qf7_go("https://page.test/a", ""), qf7_go("https://page.test/b", "")));
    qf7_drive(&engine, &fake);
    assert_eq!(fake.urls(), vec!["https://page.test/a", "https://page.test/b"]);
    let l = log.borrow().clone();
    assert_eq!(l, vec!["log:ok 200 basic AAA", "log:ok 200 basic BBB"], "answers arrive in request order");
    let e = engine.net_entries();
    assert_eq!(e.len(), 2);
    assert_eq!(e[0].outcome, "200 OK");
    assert_eq!(e[0].bytes, 3);
}

#[test]
fn qf7_policy_refusal_never_reaches_the_executor_and_rejects_async() {
    let fake = QfFake::new(vec![]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.eval("t", &format!("{}console.log('sync-done');", qf7_go("ftp://page.test/x", "")));
    assert_eq!(log.borrow().last().map(String::as_str), Some("log:sync-done"), "no rejection before the turn ends");
    qf7_drive(&engine, &fake);
    assert!(fake.urls().is_empty());
    let l = log.borrow().clone();
    assert!(l.iter().any(|x| x.starts_with("net:blocked GET ftp://page.test/x: scheme ftp:")), "{l:?}");
    assert!(l.contains(&"log:err TypeError Failed to fetch".to_string()), "{l:?}");
    assert!(engine.net_entries()[0].outcome.starts_with("blocked: "));
}

#[test]
fn qf7_private_target_warns_and_is_sent() {
    let fake = QfFake::new(vec![("127.0.0.1", qf7_ok("x", &[("access-control-allow-origin", "*")]))]);
    let (engine, log) = qf7_engine("http://page.test/", &fake);
    engine.eval("t", &qf7_go("http://127.0.0.1:8080/x", ""));
    qf7_drive(&engine, &fake);
    assert_eq!(fake.urls().len(), 1, "warn-and-allow");
    let l = log.borrow().clone();
    assert!(
        l.contains(&"net-warning:scratchy: net warning: http://page.test/ -> http://127.0.0.1:8080/x (loopback) allowed by policy".to_string()),
        "{l:?}"
    );
    assert_eq!(engine.net_entries()[0].warnings.len(), 1);
    assert!(l.contains(&"log:ok 200 cors x".to_string()), "{l:?}");
}

#[test]
fn qf7_cors_blocked_allowed_and_headers_filtered() {
    let fake = QfFake::new(vec![
        ("/closed", qf7_ok("secret", &[])),
        ("/open", qf7_ok("pub", &[("access-control-allow-origin", "*"), ("x-hidden", "1"), ("content-type", "text/plain")])),
    ]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.eval(
        "t",
        &format!(
            "{}{}{}",
            qf7_go("https://other.test/closed", ""),
            qf7_go("https://other.test/open", ""),
            "__net_request({method:'GET', url:'https://other.test/open', headers:[], body:null}).promise\
              .then(function(r){ console.log('hdr', r.headers.get('x-hidden'), r.headers.get('content-type')); });"
        ),
    );
    qf7_drive(&engine, &fake);
    let l = log.borrow().clone();
    assert!(l.contains(&"net:GET https://other.test/closed: CORS: no Access-Control-Allow-Origin header (blocked by CORS policy)".to_string()), "{l:?}");
    assert!(l.contains(&"log:err TypeError Failed to fetch".to_string()), "{l:?}");
    assert!(l.contains(&"log:ok 200 cors pub".to_string()), "{l:?}");
    assert!(l.contains(&"log:hdr null text/plain".to_string()), "non-safelisted response headers are hidden from a cors response: {l:?}");
    assert_eq!(fake.urls().len(), 3);
}

#[test]
fn qf7_no_cors_is_opaque_and_beacon_is_logged_not_sent() {
    let fake = QfFake::new(vec![]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.eval(
        "t",
        "__net_request({method:'GET', url:'https://track.test/p?x=1', headers:[], body:null, mode:'no-cors'}).promise\
           .then(function(r){ console.log('nc', r.status, r.type, JSON.stringify(r.url)); });\
         __net_request({method:'POST', url:'https://track.test/b', headers:[['Content-Type','text/plain']], body:new Uint8Array([104,105]), api:'beacon'}).promise\
           .then(function(r){ console.log('bc', r.type); });",
    );
    qf7_drive(&engine, &fake);
    assert!(fake.urls().is_empty(), "beacons are suppressed, not sent");
    let l = log.borrow().clone();
    assert!(l.contains(&"log:nc 0 opaque \"\"".to_string()), "{l:?}");
    assert!(l.contains(&"log:bc opaque".to_string()), "{l:?}");
    assert!(
        l.contains(&"net:beacon navigator.sendBeacon POST https://track.test/b content-type=text/plain body=2 bytes body[..200]=\"hi\" page=https://page.test/: suppressed (not sent)".to_string()),
        "{l:?}"
    );
    assert!(l.iter().any(|x| x.contains("beacon fetch no-cors GET https://track.test/p?x=1") && x.ends_with("suppressed (not sent)")), "{l:?}");
}

#[test]
fn qf7_beacons_send_flag_sends_and_stays_opaque() {
    let fake = QfFake::new(vec![("track.test", qf7_ok("gif", &[]))]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    let mut p = RequestPolicy::default();
    p.beacons_send = true;
    engine.set_net_policy(p);
    engine.eval("t", &qf7_go("https://track.test/p", ", mode:'no-cors'"));
    qf7_drive(&engine, &fake);
    assert_eq!(fake.urls().len(), 1);
    assert!(log.borrow().iter().any(|x| x == "log:ok 0 opaque "), "{:?}", log.borrow());
}

#[test]
fn qf7_preflight_is_rejected_with_a_clear_log() {
    let fake = QfFake::new(vec![]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.eval(
        "t",
        "__net_request({method:'PUT', url:'https://other.test/x', headers:[], body:null}).promise.catch(function(e){ console.log('err', e.name); });",
    );
    qf7_drive(&engine, &fake);
    assert_eq!(fake.urls(), vec!["https://other.test/x"], "QF13: only the preflight (which got no canned answer) was sent");
    let l = log.borrow().clone();
    assert!(l.iter().any(|x| x.starts_with("net:CORS preflight failed: https://other.test/x:")), "{l:?}");
    assert!(l.contains(&"log:err TypeError".to_string()), "{l:?}");
}

#[test]
fn qf7_abort_cancels_and_a_late_result_is_ignored() {
    let (engine, log) = qf7_engine("https://page.test/", &QfFake::new(vec![]));
    engine.eval(
        "t",
        "var c = new AbortController(); c.abort();\
         __net_request({method:'GET', url:'https://page.test/a', headers:[], body:null, signal:c.signal}).promise\
           .then(function(){ console.log('A resolved'); }, function(e){ console.log('A', e.name); });\
         var c2 = new AbortController();\
         __net_request({method:'GET', url:'https://page.test/b', headers:[], body:null, signal:c2.signal}).promise\
           .then(function(){ console.log('B resolved'); }, function(e){ console.log('B', e.name); });",
    );
    let reqs = engine.take_requests();
    assert_eq!(reqs.net.len(), 1, "the pre-aborted request is not even queued");
    let mut sched = NetScheduler::new(&RequestPolicy::default());
    let job = reqs.net.into_iter().next().unwrap();
    let id = job.id;
    sched.enqueue(job);
    let mut started = Vec::new();
    sched.pump(&mut |j| started.push(j));
    assert_eq!(started.len(), 1);
    // Abort while it is in flight, then deliver the answer anyway.
    engine.eval("t", "c2.abort();");
    assert_eq!(engine.take_requests().net_abort, vec![id]);
    engine.deliver_net(id, qf7_ok("B", &[]));
    let l = log.borrow().clone();
    assert_eq!(l, vec!["log:A AbortError", "log:B AbortError"], "{l:?}");
    // A job still queued is cancelled by its abort id.
    let mut s2 = NetScheduler::new(&RequestPolicy::default());
    s2.enqueue(started.into_iter().next().unwrap());
    assert!(s2.abort(id));
    assert_eq!(s2.counts(), (0, 0));
}

fn qf7_job(id: u64, reserve: usize) -> crate::js_net::NetJob {
    crate::js_net::NetJob { id, request: Vec::new(), limits: WireLimits::default(), reserve }
}

#[test]
fn qf7_scheduler_caps_parallelism_and_in_flight_bytes() {
    let mut p = RequestPolicy::default();
    let mut s = NetScheduler::new(&p);
    for i in 1..=10 {
        s.enqueue(qf7_job(i, 0));
    }
    let mut started = Vec::new();
    s.pump(&mut |j| started.push(j.id));
    assert_eq!(started, vec![1, 2, 3, 4, 5, 6], "max_concurrent_requests (6) from the policy");
    s.done(1);
    s.done(999); // unknown ids are a no-op
    s.pump(&mut |j| started.push(j.id));
    assert_eq!(started.len(), 7);
    assert_eq!(s.counts(), (3, 6));
    // Byte cap, FIFO: a 60-byte request waits for the other 60-byte one.
    p.max_in_flight_bytes = 100;
    let mut s = NetScheduler::new(&p);
    s.enqueue(qf7_job(1, 60));
    s.enqueue(qf7_job(2, 60));
    s.enqueue(qf7_job(3, 10));
    let mut started = Vec::new();
    s.pump(&mut |j| started.push(j.id));
    assert_eq!(started, vec![1]);
    s.done(1);
    s.pump(&mut |j| started.push(j.id));
    assert_eq!(started, vec![1, 2, 3]);
    s.reset();
    assert_eq!(s.counts(), (0, 0));
}

#[test]
fn qf7_per_page_and_body_caps_come_from_the_policy() {
    let fake = QfFake::new(vec![("page.test", qf7_ok("x", &[]))]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    let mut p = RequestPolicy::default();
    p.max_requests_per_page = 2;
    p.max_in_flight_bytes = 8;
    engine.set_net_policy(p);
    let post = |n: usize| {
        format!("__net_request({{method:'POST', url:'https://page.test/p', headers:[], body:new Uint8Array({n})}});")
    };
    engine.eval("t", &format!("{}{}{}{}", post(9), post(1), post(1), post(1)));
    qf7_drive(&engine, &fake);
    assert_eq!(fake.urls().len(), 2, "the oversized body and the third request are refused");
    let l = log.borrow().clone();
    assert!(l.iter().any(|x| x.contains("request body is too large (9 bytes, limit 8)")), "{l:?}");
    assert!(l.iter().any(|x| x.contains("too many requests from this page (limit 2)")), "{l:?}");
}

#[test]
fn qf7_sync_blocks_on_the_engine_thread_with_the_short_timeout() {
    let fake = Arc::new(QfFake {
        calls: Mutex::new(Vec::new()),
        answers: vec![("/ok", qf7_ok("sync-body", &[])), ("/slow", Err(NetError::Timeout))],
        delay_ms: 60,
        raw: Mutex::new(Vec::new()),
    });
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    let t = std::time::Instant::now();
    engine.eval(
        "t",
        "var r = __net_request({method:'GET', url:'https://page.test/ok', headers:[], body:null, sync:true}).response;\
         console.log('got', r.status, r.type);\
         try { __net_request({method:'GET', url:'https://page.test/slow', headers:[], body:null, sync:true}); }\
         catch (e) { console.log('threw', e.name); }",
    );
    assert!(t.elapsed() >= std::time::Duration::from_millis(120), "send() blocked for both requests");
    let l = log.borrow().clone();
    assert!(l.contains(&"log:got 200 basic".to_string()), "{l:?}");
    assert!(l.contains(&"log:threw NetworkError".to_string()), "{l:?}");
    assert!(l.iter().any(|x| x.starts_with("net:sync GET https://page.test/ok: blocking the page for up to 5000 ms")), "{l:?}");
    assert!(l.iter().any(|x| x.starts_with("net:GET https://page.test/slow: request timed out")), "{l:?}");
    let calls = fake.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|c| c.1 == thread::current().id()), "sync runs on the calling thread");
    assert!(calls.iter().all(|c| c.2 == std::time::Duration::from_millis(5000)), "sync_xhr_timeout_ms, not the 30 s default");
}

#[test]
fn qf7_js_report_has_a_network_section_and_uses_the_base_href() {
    let fake = QfFake::new(vec![("page.test/data", qf7_ok("12345", &[])), ("other.test", qf7_ok("no-acao", &[]))]);
    crate::js_net::set_net_executor(fake.clone());
    let html = "<html><head><base href='https://page.test/dir/'></head><body><script>\
        var q = function(u){ __net_request({method:'GET', url:u, headers:[], body:null}).promise.catch(function(){}); };\
        q('https://page.test/data'); q('https://other.test/x'); q('http://127.0.0.1:8080/');\
        </script></body></html>";
    set_js_enabled(true);
    let report = js::js_report_for_html(html, None, "/tmp/qf7-none.html");
    assert!(report.contains("--- network (3 requests, 12 bytes) ---"), "{report}");
    assert!(report.contains("GET https://page.test/data -> 200 OK 5 bytes"), "same-origin check uses the <base href>, not file://: {report}");
    assert!(report.contains("GET https://other.test/x -> blocked: CORS: no Access-Control-Allow-Origin header"), "{report}");
    assert!(report.contains("(loopback) allowed by policy"), "{report}");
    assert!(report.contains("errors total: 0"), "{report}");
}

#[test]
fn qf8_fetch_exists_on_window_and_global_this() {
    let fake = QfFake::new(vec![]);
    let l = qf8_run(
        "https://page.test/",
        &fake,
        "console.log(typeof fetch, typeof window.fetch, typeof globalThis.fetch, typeof self.fetch, fetch === window.fetch, fetch.length, fetch.name);",
    );
    assert_eq!(l, vec!["log:function function function function true 1 fetch"]);
}

#[test]
fn qf8_get_text_and_json_round_trip() {
    let fake = QfFake::new(vec![
        ("/t", qf7_ok("hello", &[("content-type", "text/plain")])),
        ("/j", qf7_ok("{\"a\":[1,2]}", &[("content-type", "application/json")])),
    ]);
    let l = qf8_run(
        "https://page.test/",
        &fake,
        "fetch('/t').then(function(r){ console.log('r', r.status, r.ok, r.type, r.url, r.redirected, r.headers.get('content-type')); return r.text(); }).then(function(t){ console.log('text', t); });\
         fetch('https://page.test/j').then(function(r){ return r.json(); }).then(function(j){ console.log('json', JSON.stringify(j)); });",
    );
    assert!(l.contains(&"log:r 200 true basic https://page.test/t false text/plain".to_string()), "{l:?}");
    assert!(l.contains(&"log:text hello".to_string()), "{l:?}");
    assert!(l.contains(&"log:json {\"a\":[1,2]}".to_string()), "{l:?}");
}

#[test]
fn qf8_post_json_body_and_headers_reach_the_executor() {
    let fake = QfFake::new(vec![("/p", qf7_ok("{}", &[]))]);
    let _ = qf8_run(
        "https://page.test/",
        &fake,
        "fetch('/p', {method:'post', headers:{'Content-Type':'application/json','X-Tok':'abc'}, body: JSON.stringify({k:1})});",
    );
    let raw = fake.raw.lock().unwrap().clone();
    assert_eq!(raw.len(), 1);
    let t = String::from_utf8_lossy(&raw[0]).to_string();
    assert!(t.starts_with("POST https://page.test/p"), "{t}");
    assert!(t.to_ascii_lowercase().contains("content-type: application/json"), "{t}");
    assert!(t.to_ascii_lowercase().contains("x-tok: abc"), "{t}");
    assert!(t.ends_with("{\"k\":1}"), "{t}");
}

#[test]
fn qf8_form_data_body_is_multipart() {
    let fake = QfFake::new(vec![("/u", qf7_ok("", &[]))]);
    let _ = qf8_run(
        "https://page.test/",
        &fake,
        "var fd = new FormData(); fd.append('a', 'one'); fd.append('f', new Blob(['xyz'], {type:'text/plain'}), 'n.txt'); fetch('/u', {method:'POST', body: fd});",
    );
    let raw = fake.raw.lock().unwrap().clone();
    let t = String::from_utf8_lossy(&raw[0]).to_string();
    assert!(t.to_ascii_lowercase().contains("content-type: multipart/form-data; boundary="), "{t}");
    assert!(t.contains("name=\"a\"") && t.contains("one") && t.contains("filename=\"n.txt\"") && t.contains("xyz"), "{t}");
}

#[test]
fn qf8_abort_before_and_after() {
    let fake = QfFake::new(vec![("/a", qf7_ok("x", &[]))]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.eval(
        "t",
        "var c = new AbortController(); c.abort();\
         fetch('/a', {signal: c.signal}).catch(function(e){ console.log('pre', e.name); });\
         var c2 = new AbortController(); c2.abort('why');\
         fetch('/a', {signal: c2.signal}).catch(function(e){ console.log('reason', e); });\
         var c3 = new AbortController();\
         fetch('/a', {signal: c3.signal}).then(function(){ console.log('B resolved'); }, function(e){ console.log('late', e.name); });\
         Promise.resolve().then(function(){ return Promise.resolve(); }).then(function(){ c3.abort(); });",
    );
    qf7_drive(&engine, &fake);
    let l = log.borrow().clone();
    assert!(l.contains(&"log:pre AbortError".to_string()), "{l:?}");
    assert!(l.contains(&"log:reason why".to_string()), "{l:?}");
    assert!(l.contains(&"log:late AbortError".to_string()), "{l:?}");
    assert!(!l.contains(&"log:B resolved".to_string()), "{l:?}");
}

#[test]
fn qf8_cors_blocked_allowed_and_opaque() {
    let fake = QfFake::new(vec![
        ("/closed", qf7_ok("secret", &[])),
        ("/open", qf7_ok("pub", &[("access-control-allow-origin", "*"), ("x-hidden", "1")])),
        ("/cred", qf7_ok("c", &[("access-control-allow-origin", "*")])),
    ]);
    let l = qf8_run(
        "https://page.test/",
        &fake,
        "fetch('https://o.test/closed').catch(function(e){ console.log('closed', e.name, e.message); });\
         fetch('https://o.test/open').then(function(r){ console.log('open', r.type, r.headers.get('x-hidden')); return r.text(); }).then(function(t){ console.log('body', t); });\
         fetch('https://o.test/cred', {credentials:'include'}).catch(function(e){ console.log('cred', e.name); });\
         fetch('https://o.test/x', {mode:'no-cors'}).then(function(r){ console.log('nc', r.type, r.status, JSON.stringify(r.url)); });",
    );
    assert!(l.contains(&"log:closed TypeError Failed to fetch".to_string()), "{l:?}");
    assert!(l.contains(&"log:open cors null".to_string()), "{l:?}");
    assert!(l.contains(&"log:body pub".to_string()), "{l:?}");
    assert!(l.contains(&"log:cred TypeError".to_string()), "credentials include rejects ACAO *: {l:?}");
    assert!(l.contains(&"log:nc opaque 0 \"\"".to_string()), "{l:?}");
}

#[test]
fn qf8_same_origin_mode_and_unsupported_schemes_reject() {
    let fake = QfFake::new(vec![]);
    let l = qf8_run(
        "https://page.test/",
        &fake,
        "fetch('https://o.test/x', {mode:'same-origin'}).catch(function(e){ console.log('so', e.name); });\
         ['ftp://h/x', 'about:blank', 'javascript:1'].forEach(function(u){ fetch(u).catch(function(e){ console.log('bad', e.name, u); }); });\
         fetch().catch(function(e){ console.log('noargs', e.name); });",
    );
    assert!(fake.urls().is_empty(), "{:?} {l:?}", fake.urls());
    assert!(l.contains(&"log:so TypeError".to_string()), "{l:?}");
    assert!(l.contains(&"log:bad TypeError ftp://h/x".to_string()), "{l:?}");
    assert!(l.contains(&"log:bad TypeError about:blank".to_string()), "{l:?}");
    assert!(l.contains(&"log:bad TypeError javascript:1".to_string()), "{l:?}");
    assert!(l.contains(&"log:noargs TypeError".to_string()), "{l:?}");
}

#[test]
fn qf8_redirect_error_and_follow_report_final_url() {
    let redirected = Ok(WireResponse {
        status: 200,
        status_text: "OK".into(),
        headers: vec![],
        body: b"final".to_vec(),
        error: None,
        final_url: Some("https://page.test/final".into()),
    });
    let fake = QfFake::new(vec![("/start", redirected)]);
    let l = qf8_run(
        "https://page.test/",
        &fake,
        "fetch('/start').then(function(r){ console.log('follow', r.url, r.redirected); });\
         fetch('/start', {redirect:'error'}).catch(function(e){ console.log('error', e.name); });",
    );
    assert!(l.contains(&"log:follow https://page.test/final true".to_string()), "{l:?}");
    assert!(l.contains(&"log:error TypeError".to_string()), "{l:?}");
}

#[test]
fn qf8_blob_and_data_urls_resolve_locally() {
    let fake = QfFake::new(vec![]);
    let l = qf8_run(
        "https://page.test/",
        &fake,
        "var u = URL.createObjectURL(new Blob(['blobby'], {type:'text/x-t'}));\
         fetch(u).then(function(r){ console.log('blob', r.status, r.type, r.headers.get('content-type'), r.url === u); return r.text(); }).then(function(t){ console.log('bt', t); });\
         fetch('data:text/plain;base64,aGk=').then(function(r){ return r.text(); }).then(function(t){ console.log('b64', t); });\
         fetch('data:,a%20b%C3%A9').then(function(r){ console.log('ct', r.headers.get('content-type')); return r.text(); }).then(function(t){ console.log('pct', t); });\
         var u3 = URL.createObjectURL(new Blob(['q'])); URL.revokeObjectURL(u3); fetch(u3).catch(function(e){ console.log('revoked', e.name); });\
         var u2 = URL.createObjectURL(new Blob(['z'])); fetch(u2).then(function(r){ return r.arrayBuffer(); }).then(function(b){ console.log('ab', b.byteLength); });",
    );
    assert!(fake.urls().is_empty(), "local fetches never reach the network");
    assert!(l.contains(&"log:blob 200 basic text/x-t true".to_string()), "{l:?}");
    assert!(l.contains(&"log:bt blobby".to_string()), "{l:?}");
    assert!(l.contains(&"log:b64 hi".to_string()), "{l:?}");
    assert!(l.contains(&"log:ct text/plain;charset=US-ASCII".to_string()), "{l:?}");
    assert!(l.contains(&"log:pct a b\u{e9}".to_string()), "{l:?}");
    assert!(l.contains(&"log:revoked TypeError".to_string()), "{l:?}");
    assert!(l.contains(&"log:ab 1".to_string()), "{l:?}");
}

#[test]
fn qf8_keepalive_beacon_is_logged_not_sent() {
    let fake = QfFake::new(vec![]);
    let l = qf8_run(
        "https://page.test/",
        &fake,
        "fetch('https://track.test/b', {method:'POST', keepalive:true, body:'x=1', headers:{'Content-Type':'text/plain'}}).then(function(r){ console.log('ka', r.type, r.status); });",
    );
    assert!(fake.urls().is_empty());
    assert!(l.contains(&"log:ka opaque 0".to_string()), "{l:?}");
    assert!(l.iter().any(|x| x.starts_with("net:") && x.contains("suppressed (not sent)")), "{l:?}");
}

#[test]
fn qf8_network_failure_rejects_failed_to_fetch() {
    let fake = QfFake::new(vec![]);
    let l = qf8_run("https://page.test/", &fake, "fetch('/nothing').catch(function(e){ console.log('net', e.name, e.message); });");
    assert!(l.contains(&"log:net TypeError Failed to fetch".to_string()), "{l:?}");
}

// ---------- QF10: navigator.sendBeacon (fake executor, no network) ----------

#[test]
fn qf10_send_beacon_exists_on_navigator_and_prototype() {
    let fake = QfFake::new(vec![]);
    let l = qf8_run(
        "https://page.test/",
        &fake,
        "console.log(typeof navigator.sendBeacon, typeof Navigator, Navigator.prototype.sendBeacon === navigator.sendBeacon, navigator instanceof Navigator, navigator.userAgent.length > 0);",
    );
    assert_eq!(l, vec!["log:function function true true true"], "{l:?}");
}

#[test]
fn qf10_send_beacon_is_logged_not_sent_and_returns_true() {
    let fake = QfFake::new(vec![]);
    let l = qf8_run(
        "https://page.test/dir/",
        &fake,
        "console.log('r', navigator.sendBeacon('t?x=1', 'hello'));\
         console.log('u', navigator.sendBeacon('https://b.test/u', new URLSearchParams({a:'b c'})));\
         console.log('n', navigator.sendBeacon('https://b.test/n'));\
         console.log('a', navigator.sendBeacon('https://b.test/a', new Uint8Array([1,2,3])));\
         console.log('b', navigator.sendBeacon('https://b.test/b', new Blob(['{}'], {type:'application/json'})));\
         var fd = new FormData(); fd.append('k','v'); console.log('f', navigator.sendBeacon('https://b.test/f', fd));",
    );
    assert!(fake.urls().is_empty(), "suppressed beacons never reach the transport");
    for t in ["r", "u", "n", "a", "b", "f"] {
        assert!(l.contains(&format!("log:{t} true")), "{t}: {l:?}");
    }
    let net: Vec<&String> = l.iter().filter(|x| x.starts_with("net:beacon")).collect();
    assert_eq!(net.len(), 6, "{l:?}");
    assert!(net[0].contains("navigator.sendBeacon POST https://page.test/dir/t?x=1 content-type=text/plain;charset=UTF-8 body=5 bytes body[..200]=\"hello\" page=https://page.test/dir/"), "{}", net[0]);
    assert!(net[0].ends_with("suppressed (not sent)"), "{}", net[0]);
    assert!(net[1].contains("content-type=application/x-www-form-urlencoded;charset=UTF-8 body=5 bytes"), "{}", net[1]);
    assert!(net[2].contains("body=0 bytes"), "{}", net[2]);
    assert!(net[3].contains("body=3 bytes"), "{}", net[3]);
    assert!(net[4].contains("content-type=application/json body=2 bytes"), "{}", net[4]);
    assert!(net[5].contains("content-type=multipart/form-data; boundary="), "{}", net[5]);
}

#[test]
fn qf10_send_beacon_validates_the_url_and_the_size() {
    let fake = QfFake::new(vec![]);
    let l = qf8_run(
        "https://page.test/",
        &fake,
        "['ftp://x.test/', 'javascript:1', 'data:text/plain,hi', 'http://[bad'].forEach(function (u) {\
           try { navigator.sendBeacon(u); console.log('no-throw', u); } catch (e) { console.log('thrown', e.name); }\
         });\
         try { navigator.sendBeacon(); } catch (e) { console.log('noarg', e.name); }\
         console.log('big', navigator.sendBeacon('https://b.test/x', new Uint8Array(65 * 1024)));\
         console.log('ok', navigator.sendBeacon('https://b.test/x', new Uint8Array(64 * 1024)));",
    );
    assert_eq!(l.iter().filter(|x| *x == "log:thrown TypeError").count(), 4, "{l:?}");
    assert!(l.contains(&"log:noarg TypeError".to_string()), "{l:?}");
    assert!(l.contains(&"log:big false".to_string()), "{l:?}");
    assert!(l.contains(&"log:ok true".to_string()), "{l:?}");
    assert_eq!(l.iter().filter(|x| x.starts_with("net:beacon")).count(), 1, "an oversized beacon is not even logged as sent: {l:?}");
}

#[test]
fn qf10_send_beacon_goes_through_the_policy_and_the_beacons_send_flag() {
    let fake = QfFake::new(vec![("track.test", qf7_ok("", &[])), ("127.0.0.1", qf7_ok("", &[]))]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    let mut p = RequestPolicy::default();
    p.beacons_send = true;
    engine.set_net_policy(p);
    engine.eval("t", "console.log(navigator.sendBeacon('https://track.test/b', 'hi')); console.log(navigator.sendBeacon('http://127.0.0.1:9/p', 'x'));");
    qf7_drive(&engine, &fake);
    let urls = fake.urls();
    assert!(urls.iter().any(|u| u.contains("track.test/b")), "beacons_send sends through the same executor: {urls:?}");
    let raw = String::from_utf8_lossy(&fake.raw.lock().unwrap()[0]).to_string();
    assert!(raw.contains("POST") && raw.to_ascii_lowercase().contains("content-type: text/plain;charset=utf-8"), "{raw}");
    let l = log.borrow().clone();
    assert!(!l.iter().any(|x| x.starts_with("net:beacon")), "{l:?}");
}

// ---------- QF7a: fail closed on an old wire, target-class warnings ----------

#[test]
fn qf7a_old_wire_fails_closed_with_one_log_line() {
    let fake = QfFake::new(vec![("/a", qf7_ok("AAA", &[])), ("/b", qf7_ok("BBB", &[]))]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.set_check_wire_version(true);
    engine.eval("t", &format!("{}{}{}", qf7_go("https://page.test/a", ""), qf7_go("https://page.test/b", ""), qf7_go("https://page.test/c", "")));
    qf7_drive(&engine, &fake);
    let l = log.borrow().clone();
    assert_eq!(l.iter().filter(|x| *x == "net-warning:scratchy: page networking disabled: wire is too old (restart it)").count(), 1, "{l:?}");
    assert_eq!(l.iter().filter(|x| x.starts_with("log:err")).count(), 3, "every request fails: {l:?}");
    assert!(!l.iter().any(|x| x.starts_with("log:ok")), "{l:?}");
    assert!(!l.iter().any(|x| x.starts_with("net:")), "no further log lines: {l:?}");
    // Requests sent in the same batch are already in flight; a later one is refused before the executor.
    engine.eval("t2", &qf7_go("https://page.test/d", ""));
    qf7_drive(&engine, &fake);
    assert!(!fake.urls().iter().any(|u| u.ends_with("/d")), "nothing is sent after the old wire was found: {:?}", fake.urls());
    assert_eq!(log.borrow().iter().filter(|x| x.starts_with("log:err")).count(), 4);
    assert!(engine.net_entries().iter().all(|e| e.outcome.starts_with("blocked: page networking disabled")), "{:?}", engine.net_entries());
}

#[test]
fn qf7a_wire_v2_and_v3_keep_networking_working() {
    let fake = QfFake::new(vec![("/a", qf7_ok("AAA", &[("x-wire-version", "2")])), ("/b", qf7_ok("BBB", &[("x-wire-version", "3"), ("x-wire-cookies", "on")]))]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.set_check_wire_version(true);
    engine.eval("t", &format!("{}{}", qf7_go("https://page.test/a", ""), qf7_go("https://page.test/b", "")));
    qf7_drive(&engine, &fake);
    let l = log.borrow().clone();
    assert_eq!(l, vec!["log:ok 200 basic AAA", "log:ok 200 basic BBB"], "{l:?}");
}

#[test]
fn qf7a_wire_control_headers_are_hidden_from_the_page() {
    let fake = QfFake::new(vec![("/a", qf7_ok("x", &[("x-wire-version", "3"), ("x-wire-target-class", "public"), ("x-custom", "1")]))]);
    let l = qf8_run("https://page.test/", &fake, "fetch('/a').then(function(r){ console.log('h', r.headers.get('x-wire-version'), r.headers.get('x-wire-target-class'), r.headers.get('x-custom')); });");
    assert!(l.contains(&"log:h null null 1".to_string()), "{l:?}");
}

#[test]
fn qf7a_dns_name_resolving_privately_warns_once() {
    let fake = QfFake::new(vec![
        ("rebind.test", qf7_ok("x", &[("x-wire-target-class", "private")])),
        ("fine.test", qf7_ok("x", &[("x-wire-target-class", "public")])),
        ("127.0.0.1", qf7_ok("x", &[("x-wire-target-class", "loopback")])),
    ]);
    let l = qf8_run(
        "https://page.test/",
        &fake,
        "fetch('https://rebind.test/x'); fetch('https://fine.test/x'); fetch('http://127.0.0.1:8080/x');",
    );
    let w: Vec<&String> = l.iter().filter(|x| x.starts_with("net-warning:")).collect();
    assert_eq!(w.len(), 2, "{l:?}");
    assert!(w.iter().any(|x| x.contains("https://rebind.test/x (private) allowed by policy")), "{w:?}");
    assert_eq!(w.iter().filter(|x| x.contains("127.0.0.1")).count(), 1, "a literal IP is warned about by the URL check only: {w:?}");
}

#[test]
fn qf7a_block_mode_turns_wire_policy_refusal_into_a_clear_error() {
    let refusal = Ok(WireResponse {
        status: 403,
        status_text: "Forbidden".into(),
        error: Some("wire: blocked: rebind.test resolves to a non-public address (10.0.0.5)".into()),
        ..Default::default()
    });
    let fake = QfFake::new(vec![("rebind.test", refusal)]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    let mut p = RequestPolicy::default();
    p.private_targets = PrivatePolicy::Block;
    engine.set_net_policy(p);
    engine.eval("t", &qf7_go("https://rebind.test/x", ""));
    qf7_drive(&engine, &fake);
    let l = log.borrow().clone();
    assert!(l.iter().any(|x| x.starts_with("net:") && x.contains("rebind.test resolves to a non-public address (private_targets is block)")), "{l:?}");
    assert!(l.contains(&"log:err TypeError Failed to fetch".to_string()), "{l:?}");
}

#[test]
fn qf7a_suppressed_beacons_and_refusals_do_not_trip_the_old_wire_check() {
    let fake = QfFake::new(vec![]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.set_check_wire_version(true);
    engine.eval(
        "t",
        "navigator.sendBeacon('https://track.test/b', 'x'); fetch('ftp://x.test/').catch(function(){ console.log('refused'); });\
         fetch('https://track.test/k', {method:'POST', keepalive:true, body:'y'}).then(function(r){ console.log('ka', r.type); });",
    );
    qf7_drive(&engine, &fake);
    let l = log.borrow().clone();
    assert!(!l.iter().any(|x| x.contains("page networking disabled")), "{l:?}");
    assert!(l.contains(&"log:ka opaque".to_string()), "{l:?}");
    assert_eq!(l.iter().filter(|x| x.starts_with("net:beacon")).count(), 2, "{l:?}");
}

// ---------- QF9a: DOMParser + XHR responseType 'document' ----------

#[test]
fn qf9a_domparser_html_is_an_inert_second_document() {
    let log = run_in_html(
        "<html><body><p id=x>page</p></body></html>",
        "var d=new DOMParser().parseFromString('<!doctype html><title>T</title><div id=a class=c>hi <b>there</b></div><form id=f><a href=/z>l</a></form><script>window.__ran=1</script><noscript><i>ns</i></noscript>','text/html');\
         console.log(d!==document, d.contentType, d.URL, d.defaultView, d.title, d.documentElement.tagName, d.head.tagName, d.body.tagName);\
         console.log(d.getElementById('a').textContent, d.querySelector('.c b').textContent, d.querySelectorAll('div').length, d.getElementById('x'), d.getElementById('a').innerHTML);\
         console.log(d.forms.length, d.links.length, d.querySelector('noscript i').textContent, typeof window.__ran);\
         var n=document.importNode(d.getElementById('a'),true); document.body.appendChild(n);\
         console.log(n.ownerDocument===document, document.getElementById('a').textContent, d.getElementById('a')!==null);\
         var m=d.querySelector('a'); document.body.appendChild(document.adoptNode(m)); console.log(m.ownerDocument===document, d.links.length);",
    );
    assert_eq!(
        log,
        [
            "log:true text/html about:blank null T HTML HEAD BODY",
            "log:hi there there 1 null hi <b>there</b>",
            "log:1 1 ns undefined",
            "log:true hi there true",
            "log:true 0"
        ],
        "{log:?}"
    );
}

#[test]
fn qf9a_domparser_xml_svg_and_parsererror() {
    let log = run_in_html(
        "<html><body></body></html>",
        "var p=new DOMParser();\
         var x=p.parseFromString('<?xml version=\"1.0\"?><root a=\"1\"><item>one</item><item>two<![CDATA[<&>]]></item></root>','application/xml');\
         console.log(x.contentType, x.documentElement.tagName, x.documentElement.getAttribute('a'), x.getElementsByTagName('item').length, x.querySelectorAll('item')[1].textContent, x.documentElement.firstChild.textContent);\
         var s=p.parseFromString('<svg xmlns=\"http://www.w3.org/2000/svg\"><rect width=\"3\"/></svg>','image/svg+xml');\
         console.log(s.contentType, s.documentElement.namespaceURI, s.querySelector('rect').getAttribute('width'));\
         var h=p.parseFromString('<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p id=\"q\">t</p></body></html>','application/xhtml+xml');\
         console.log(h.contentType, h.getElementById('q').textContent);\
         var b=p.parseFromString('<a><b></a>','text/xml');\
         console.log(b.contentType, b.getElementsByTagName('parsererror').length, b.documentElement.tagName);\
         try { p.parseFromString('x','text/plain'); } catch (e) { console.log(e.name); }\
         try { p.parseFromString('x'); } catch (e) { console.log(e.name); }",
    );
    assert_eq!(
        log,
        [
            "log:application/xml root 1 2 two<&> one",
            "log:image/svg+xml http://www.w3.org/2000/svg 3",
            "log:application/xhtml+xml t",
            "log:text/xml 1 parsererror",
            "log:TypeError",
            "log:TypeError"
        ],
        "{log:?}"
    );
}

#[test]
fn qf9a_xhr_response_type_document_and_response_xml() {
    let fake = QfFake::new(vec![
        ("/h", qf7_ok("<html><body><p id=k>hello</p><script>window.__ran=1</script></body></html>", &[("content-type", "text/html; charset=utf-8")])),
        ("/x", qf7_ok("<r><i>1</i></r>", &[("content-type", "application/xml")])),
        ("/t", qf7_ok("plain", &[("content-type", "text/plain")])),
        ("/e", qf7_ok("<r><i></r>", &[("content-type", "text/xml")])),
    ]);
    let l = qf9_run(
        "https://page.test/",
        &fake,
        "function get(u, rt, tag) { var x = new XMLHttpRequest(); x.open('GET', u); if (rt) x.responseType = rt;\
           x.onload = function () { var d = rt === 'document' ? x.response : x.responseXML;\
             console.log(tag, d === null ? 'null' : d.contentType + ' ' + d.documentElement.tagName + ' ' + (d.getElementById && d.getElementById('k') ? d.getElementById('k').textContent : d.textContent), x.responseXML === d, typeof window.__ran); }; x.send(); }\
         get('/h', 'document', 'html'); get('/x', 'document', 'xml'); get('/x', '', 'xml2'); get('/h', '', 'htmlplain'); get('/t', 'document', 'txt'); get('/t', '', 'txt2'); get('/e', '', 'bad');",
    );
    assert!(l.contains(&"log:html text/html HTML hello true undefined".to_string()), "{l:?}");
    assert!(l.contains(&"log:xml application/xml r 1 true undefined".to_string()), "{l:?}");
    assert!(l.contains(&"log:xml2 application/xml r 1 true undefined".to_string()), "{l:?}");
    assert!(l.contains(&"log:htmlplain null true undefined".to_string()), "{l:?}");
    assert!(l.contains(&"log:txt null true undefined".to_string()), "{l:?}");
    assert!(l.contains(&"log:txt2 null true undefined".to_string()), "{l:?}");
    assert!(l.iter().any(|s| s.starts_with("log:bad text/xml parsererror")), "{l:?}");
}

// QM1g/QM1g1: video painting and the video-rect-only redraw. The pure parts and the
// host plumbing run in the default build (no libmpv).
mod qm1g_video {
    use super::*;
    use crate::css::ObjectFit;
    use crate::media_video::*;
    use std::sync::Arc;

    fn frame(w: u32, h: u32, v: u32) -> Arc<VideoFrame> {
        Arc::new(VideoFrame::new(w, h, vec![v; (w * h) as usize]))
    }

    #[test]
    fn clip_rect_cuts_at_window_and_ancestor_clip() {
        assert_eq!(clip_rect((10, 10, 100, 50), None, 200, 200), Some((10, 10, 100, 50)));
        // scrolled partly above the window and past the right edge
        assert_eq!(clip_rect((150, -20, 100, 50), None, 200, 200), Some((150, 0, 50, 30)));
        // an ancestor's overflow clip
        assert_eq!(clip_rect((10, 10, 100, 50), Some((0.0, 0.0, 60.0, 30.0)), 200, 200), Some((10, 10, 50, 20)));
        // fully outside
        assert_eq!(clip_rect((10, 300, 100, 50), None, 200, 200), None);
        assert_eq!(clip_rect((10, 10, 100, 50), Some((120.0, 0.0, 10.0, 10.0)), 200, 200), None);
    }

    #[test]
    fn letterbox_math_16x9_in_4x3() {
        // contain: full width, bars above and below (the black letterbox)
        let (x, y, w, h) = fit_rect(400.0, 300.0, 1280.0, 720.0, ObjectFit::Contain);
        assert_eq!((x, y, w, h), (0.0, 37.5, 400.0, 225.0));
        // fill stretches, cover crops (wider than the box), none keeps the size
        assert_eq!(fit_rect(400.0, 300.0, 1280.0, 720.0, ObjectFit::Fill), (0.0, 0.0, 400.0, 300.0));
        let (x, _, w, h) = fit_rect(400.0, 300.0, 1280.0, 720.0, ObjectFit::Cover);
        assert!((h - 300.0).abs() < 0.01 && w > 400.0 && x < 0.0);
        assert_eq!(fit_rect(400.0, 300.0, 100.0, 50.0, ObjectFit::None), (150.0, 125.0, 100.0, 50.0));
        // scale-down never enlarges
        assert_eq!(fit_rect(400.0, 300.0, 100.0, 50.0, ObjectFit::ScaleDown), (150.0, 125.0, 100.0, 50.0));
        assert_eq!(fit_rect(0.0, 300.0, 100.0, 50.0, ObjectFit::Contain), (0.0, 0.0, 0.0, 0.0));
    }

    #[test]
    fn visibility_rules() {
        assert!(wants_render(10, false, false));
        assert!(!wants_render(0, false, false), "scrolled away or fully covered: no rendering");
        assert!(!wants_render(10, false, true), "the page is hidden (:help)");
        assert!(wants_render(0, true, false), "a layered box is on screen");
    }

    /// A 20x10 window with a sentinel video rect (4,2) 10x6 and an overlay at (8,2) 3x6.
    fn sentinel_page() -> (Vec<u32>, IRect) {
        let mut px = vec![0x00FF_FFFFu32; 20 * 10];
        let vis: IRect = (4, 2, 10, 6);
        fill(&mut px, 20, vis, SENTINEL);
        for y in 2..8 {
            for x in 8..11 {
                px[y * 20 + x] = 0x0000_FF00; // content painted above the video
            }
        }
        (px, vis)
    }

    #[test]
    fn overlay_mask_marks_only_untouched_video_pixels() {
        let (px, vis) = sentinel_page();
        let m = overlay_mask(&px, 20, vis);
        assert_eq!(m.len(), 60);
        assert_eq!(m.iter().filter(|&&b| b).count(), 60 - 18);
        assert!(m[0] && !m[4] && !m[6] && m[7]);
        assert_eq!(mask_bbox(&m, vis), Some(vis));
        // a half-transparent paint over a sentinel pixel also kills it (blend writes top byte 0)
        let mut p2 = px.clone();
        p2[2 * 20 + 4] = crate::paint::blend(p2[2 * 20 + 4], TextColor::rgba(255, 0, 0, 128));
        assert!(!overlay_mask(&p2, 20, vis)[0]);
    }

    #[test]
    fn finish_composites_frame_through_mask_and_leaves_the_rest() {
        let (mut px, vis) = sentinel_page();
        let before = px.clone();
        set_frames([(7usize, frame(10, 6, 0x0012_3456))].into_iter().collect());
        let b = PaintedBox { node: 7, rect: vis, vis: Some(vis), layered: false, fit: ObjectFit::Contain, pos: ((0.5, 0.0), (0.5, 0.0)), poster: None };
        let infos = finish(vec![b], &mut px, 20);
        assert_eq!(infos.len(), 1);
        for y in 0..10usize {
            for x in 0..20usize {
                let i = y * 20 + x;
                let in_video = (4..14).contains(&x) && (2..8).contains(&y);
                let covered = (8..11).contains(&x) && (2..8).contains(&y);
                if in_video && !covered {
                    assert_eq!(px[i], 0x0012_3456, "({x},{y})");
                } else {
                    assert_eq!(px[i], before[i], "outside the video or above it ({x},{y})");
                }
            }
        }
        // a new frame: only the unobstructed pixels change, damage = bounding rect
        let f2 = frame(10, 6, 0x0065_4321);
        let d = update_box(&infos[0], &f2, &mut px, 20);
        assert_eq!(d, Some(vis));
        assert_eq!(px[2 * 20 + 4], 0x0065_4321);
        assert_eq!(px[2 * 20 + 8], 0x0000_FF00, "the overlay stays");
        // a frame of another size is refused (full redraw)
        assert_eq!(update_box(&infos[0], &frame(9, 6, 1), &mut px, 20), None);
        set_frames(Default::default());
    }

    /// QM1k1n6c: the kept page's boxes carry the rect that is on screen; a frame goes there
    /// (not at a rect a pending relayout/scroll will produce), and a dirty layout does not
    /// refuse the damage present.
    #[test]
    fn dirty_layout_frame_lands_at_the_painted_rect() {
        assert!(crate::app::video_damage_ok((20, 10), 200, (20, 10)));
        assert!(!crate::app::video_damage_ok((20, 10), 200, (30, 10)));
        assert!(!crate::app::video_damage_ok((20, 10), 0, (20, 10)));
        let mut px = vec![0x00FF_FFFFu32; 20 * 20];
        let at = |y: i32| PaintedBox { node: 7, rect: (4, y, 10, 6), vis: Some((4, y, 10, 6)), layered: false, fit: ObjectFit::Contain, pos: ((0.5, 0.0), (0.5, 0.0)), poster: None };
        set_frames([(7usize, frame(10, 6, 0x0012_3456))].into_iter().collect());
        fill(&mut px, 20, (4, 2, 10, 6), SENTINEL);
        let infos = finish(vec![at(2)], &mut px, 20);
        let d = update_box(&infos[0], &frame(10, 6, 0x0065_4321), &mut px, 20);
        assert_eq!(d, Some((4, 2, 10, 6)));
        assert_eq!(px[2 * 20 + 4], 0x0065_4321);
        // after a scroll the full redraw repaints and the new boxes carry the scrolled rect
        let mut px2 = vec![0x00FF_FFFFu32; 20 * 20];
        fill(&mut px2, 20, (4, 9, 10, 6), SENTINEL);
        let infos2 = finish(vec![at(9)], &mut px2, 20);
        let d2 = update_box(&infos2[0], &frame(10, 6, 0x0011_1111), &mut px2, 20);
        assert_eq!(d2, Some((4, 9, 10, 6)));
        assert_eq!(px2[9 * 20 + 4], 0x0011_1111);
        assert_eq!(px2[2 * 20 + 4], 0x00FF_FFFF, "nothing at the old position");
        set_frames(Default::default());
    }

    #[test]
    fn no_frame_paints_black_and_obstruction_shrinks_the_damage() {
        set_frames(Default::default());
        let mut px = vec![0x00FF_FFFFu32; 20 * 10];
        let vis: IRect = (0, 0, 10, 4);
        fill(&mut px, 20, vis, SENTINEL);
        // cover the whole right half: the damage rect shrinks to the left half
        for y in 0..4 {
            for x in 5..10 {
                px[y * 20 + x] = 0x00AA_AAAA;
            }
        }
        let b = PaintedBox { node: 1, rect: vis, vis: Some(vis), layered: false, fit: ObjectFit::Contain, pos: ((0.5, 0.0), (0.5, 0.0)), poster: None };
        let infos = finish(vec![b], &mut px, 20);
        assert_eq!(px[0], 0, "black while there is no frame and no poster");
        assert_eq!(infos[0].bbox, Some((0, 0, 5, 4)));
        assert_eq!(infos[0].visible_pixels(), 20);
    }

    #[test]
    fn fully_covered_box_is_not_wanted_and_has_no_damage() {
        let mut px = vec![0u32; 20 * 10];
        let vis: IRect = (0, 0, 4, 4);
        fill(&mut px, 20, vis, SENTINEL);
        fill(&mut px, 20, vis, 0x0011_1111); // something opaque on top
        let b = PaintedBox { node: 2, rect: vis, vis: Some(vis), layered: false, fit: ObjectFit::Fill, pos: ((0.5, 0.0), (0.5, 0.0)), poster: None };
        let infos = finish(vec![b], &mut px, 20);
        assert_eq!(infos[0].bbox, None);
        assert!(!wants_render(infos[0].visible_pixels(), infos[0].layered, false));
        assert_eq!(update_box(&infos[0], &frame(4, 4, 9), &mut px, 20), None);
    }

    fn find_tag(h: &Handle, tag: &str) -> Option<Handle> {
        if let NodeData::Element { name, .. } = &h.data
            && name.local.as_ref() == tag
        {
            return Some(h.clone());
        }
        h.children.borrow().iter().find_map(|c| find_tag(c, tag))
    }

    /// Lays out and paints `html` into a 200x200 window like `App::redraw` does.
    fn paint_video_page(html: &str, scroll: f32, with_frame: bool) -> (Vec<u32>, Vec<VideoBoxInfo>) {
        let doc = parse_html(html);
        let node = node_id(&find_tag(&doc, "video").unwrap());
        set_boxes_enabled(Some(true));
        set_frames(if with_frame { [(node, frame(100, 50, 0x00AB_CDEF))].into_iter().collect() } else { Default::default() });
        let tree = build_layout_tree_sized(&doc, 200.0, 200.0);
        let mut font_system = new_font_system();
        let mut cache = ShapeCache::new();
        let mut next_id = 0usize;
        let mut incomplete = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let laid = measure_document(&tree, &mut font_system, 200.0, 200.0, &mut next_id, &mut cache, deadline, &mut incomplete);
        let mut swash = SwashCache::new();
        let mut pixels = vec![0x00FF_FFFFu32; 200 * 200];
        begin_paint(&pixels);
        paint(&laid, &mut font_system, &mut swash, &mut cache, &HashMap::new(), &mut pixels, 200, 200, scroll, None);
        let painted = end_paint();
        let infos = finish(painted, &mut pixels, 200);
        set_boxes_enabled(None);
        set_frames(Default::default());
        (pixels, infos)
    }

    const VIDEO_PAGE: &str = "<html><head><style>body{margin:0}video{display:block}\
        #o{position:absolute;left:20px;top:30px;width:30px;height:20px;background:#00ff00}</style></head>\
        <body><div style='height:20px'></div><video width=100 height=50></video><div id=o></div><div style='height:400px'></div></body></html>";

    #[test]
    fn video_box_is_a_replaced_box_and_obeys_paint_order() {
        let (px, infos) = paint_video_page(VIDEO_PAGE, 0.0, true);
        assert_eq!(infos.len(), 1);
        let i = &infos[0];
        assert_eq!(i.rect, (0, 20, 100, 50), "attributes size the box, below the 20px spacer");
        // the video shows, the absolutely positioned green box paints ABOVE it
        assert_eq!(px_at(&px, 200, 5, 25), 0x00AB_CDEF);
        assert_eq!(px_at(&px, 200, 30, 40), 0x0000_FF00, "above the video: the overlay");
        assert_eq!(px_at(&px, 200, 150, 25), 0x00FF_FFFF, "outside the video: untouched");
        assert_eq!(i.visible_pixels(), 5000 - 30 * 20);
        assert_eq!(i.target(), VideoTarget { w: 100, h: 50, fit: ObjectFit::Contain });
    }

    #[test]
    fn scrolling_moves_and_clips_the_video_like_an_image() {
        let (_, infos) = paint_video_page(VIDEO_PAGE, 45.0, false);
        let i = &infos[0];
        assert_eq!(i.rect, (0, -25, 100, 50));
        assert_eq!(i.vis, (0, 0, 100, 25), "clipped at the top edge of the window");
        let (_, gone) = paint_video_page(VIDEO_PAGE, 100.0, false);
        assert!(gone.is_empty(), "scrolled out of view: no box, so no rendering");
    }

    /// QM1k1n1: YouTube parks the video at `top:-360px` inside an `overflow:hidden` player: the
    /// box is laid and reached by paint, but nothing of it is visible. It must still be reported
    /// as laid (so a playing element keeps decoding); display:none has no box at all.
    #[test]
    fn qm1k1n1_paint_reports_a_laid_box_that_the_clip_hides() {
        fn painted(html: &str) -> Vec<PaintedBox> {
            let doc = parse_html(html);
            set_boxes_enabled(Some(true));
            let tree = build_layout_tree_sized(&doc, 700.0, 500.0);
            let mut font_system = new_font_system();
            let mut cache = ShapeCache::new();
            let mut next_id = 0usize;
            let mut incomplete = false;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let laid = measure_document(&tree, &mut font_system, 700.0, 500.0, &mut next_id, &mut cache, deadline, &mut incomplete);
            let mut swash = SwashCache::new();
            let mut pixels = vec![0x00FF_FFFFu32; 700 * 500];
            begin_paint(&pixels);
            paint(&laid, &mut font_system, &mut swash, &mut cache, &HashMap::new(), &mut pixels, 700, 500, 0.0, None);
            let out = end_paint();
            set_boxes_enabled(None);
            out
        }
        let page = |top: &str, extra: &str| {
            format!("<html><head><style>body{{margin:0}}#p{{position:relative;overflow:hidden;width:640px;height:360px;margin:80px 24px}}\
                video{{position:absolute;left:0;{extra}}}</style></head><body><div id=p><video style='width:640px;height:360px;top:{top}'></video></div></body></html>")
        };
        let hidden = painted(&page("-360px", ""));
        assert_eq!(hidden.len(), 1, "a laid but clipped video is still reported");
        assert!(hidden[0].vis.is_none(), "{:?}", hidden[0].vis);
        assert_eq!(hidden[0].rect.3, 360);
        // in view: the same element, visible
        let shown = painted(&page("0px", ""));
        assert_eq!(shown.len(), 1);
        assert!(shown[0].vis.is_some());
        // no box: display:none
        assert!(painted(&page("0px", "display:none")).is_empty(), "display:none has no box");
        // finish() lists only visible boxes for rendering
        let mut px = vec![0u32; 700 * 500];
        assert!(finish(hidden, &mut px, 700).is_empty());
    }

    /// QM1g2d: the bar is ordinary pixels in the video's bottom 32 px, over the picture.
    #[test]
    fn qm1g2d_bar_paints_in_the_bottom_32px_and_the_damage_is_the_bar_only() {
        use crate::media_controls::{paint, ControlsState};
        let html = "<html><head><style>body{margin:0}video{display:block}</style></head><body><video width=100 height=50></video></body></html>";
        let (mut px, infos) = paint_video_page(html, 0.0, true);
        let i = &infos[0];
        assert_eq!(i.rect, (0, 0, 100, 50));
        let before = px.clone();
        let st = ControlsState { duration: 20.0, ..Default::default() };
        paint(&mut px, 200, i, &st, false);
        assert_eq!(px, before, "a hidden bar changes nothing (no big button in a 50 px box)");
        paint(&mut px, 200, i, &st, true);
        assert_ne!(px_at(&px, 200, 5, 45), 0x00AB_CDEF, "a visible bar darkens the bottom rows");
        assert_eq!(px_at(&px, 200, 5, 10), 0x00AB_CDEF, "the picture above the bar is untouched");
        // the bar-only redraw puts the picture back inside the bar rect and nowhere else
        set_frames([(i.node, frame(100, 50, 0x00AB_CDEF))].into_iter().collect());
        let r = recomposite(i, (0, 18, 100, 32), &mut px, 200).unwrap();
        assert_eq!(r, (0, 18, 100, 32));
        assert_eq!(px, before, "recompositing the bar rect restores exactly the frame");
        assert_eq!(recomposite(i, (150, 0, 10, 10), &mut px, 200), None);
        set_frames(Default::default());
    }

    /// QM1n1: with video boxes off (no `media` feature / no libmpv) `<video>` is still an
    /// inert replaced box; returns its (width, height) in the laid-out tree.
    fn inert_video_size(html: &str) -> Option<(f32, f32)> {
        set_boxes_enabled(Some(false));
        let doc = parse_html(html);
        let tree = build_layout_tree_sized(&doc, 800.0, 600.0);
        let mut font_system = new_font_system();
        let mut cache = ShapeCache::new();
        let mut next_id = 0usize;
        let mut incomplete = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let laid = measure_document(&tree, &mut font_system, 800.0, 600.0, &mut next_id, &mut cache, deadline, &mut incomplete);
        set_boxes_enabled(None);
        fn find(l: &Laid) -> Option<(f32, f32)> {
            if matches!(l.content, LaidContent::Image(_)) {
                return Some((l.w, l.h));
            }
            match &l.content {
                LaidContent::Blocks(c) => c.iter().find_map(find),
                LaidContent::MixedInline(items) => items.iter().find_map(|i| match i {
                    MixedPositioned::Embedded(b) => find(b),
                    _ => None,
                }),
                _ => None,
            }
        }
        find(&laid)
    }

    #[test]
    fn qm1n1_inert_video_box_with_attributes() {
        let sz = inert_video_size("<body style='margin:0'><video width=640 height=360><source src=a.mp4></video></body>");
        assert_eq!(sz, Some((640.0, 360.0)));
    }

    #[test]
    fn qm1n1_inert_video_default_300x150() {
        assert_eq!(inert_video_size("<body style='margin:0'><video></video></body>"), Some((300.0, 150.0)));
    }

    #[test]
    fn qm1n1_inert_video_fallback_text_not_laid_out() {
        let with = inert_video_size("<body style='margin:0'><video width=100 height=50>fallback text that is long</video></body>");
        assert_eq!(with, Some((100.0, 50.0)));
    }

    #[test]
    fn qm1n1_inert_video_poster_sizes_the_box() {
        // 1x1 PNG poster as a data: URI (decoded at build time)
        let png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
        let sz = inert_video_size(&format!("<body style='margin:0'><video poster=\"{png}\"></video></body>"));
        assert_eq!(sz, Some((1.0, 1.0)));
    }

    #[test]
    fn host_hands_targets_to_backends_and_renders_only_wanted_frames() {
        use crate::media_el::*;
        let h = fake_new();
        let h2 = h.clone();
        set_backend_factory(Some(Rc::new(move |_: &BackendCtx| -> Box<dyn MediaBackend> { Box::new(FakeBackend(h2.clone())) })));
        let mut host = MediaHost::new(BackendCtx { socket: None, policy: Default::default() }, Rc::new(|_, _| {}));
        set_backend_factory(None);
        host.core(5);
        host.set_targets(&[(5, VideoTarget { w: 320, h: 180, fit: ObjectFit::Contain })].into_iter().collect());
        assert!(h.borrow().calls.contains(&"target Some((320, 180))".to_string()));
        h.borrow_mut().pending_frame = true;
        assert_eq!(host.render_frames(), vec![5]);
        let st = host.video_state();
        let f = st[0].1.as_ref().unwrap();
        assert_eq!((f.w, f.h), (320, 180));
        // same target again: no second call; no pending frame: nothing rendered
        let n = h.borrow().calls.len();
        host.set_targets(&[(5, VideoTarget { w: 320, h: 180, fit: ObjectFit::Contain })].into_iter().collect());
        assert_eq!(h.borrow().calls.len(), n);
        assert!(host.render_frames().is_empty());
        // hidden: the target goes away, a queued frame is consumed but not rendered
        host.set_targets(&Default::default());
        assert_eq!(h.borrow().calls.last().unwrap(), "target None");
        h.borrow_mut().pending_frame = true;
        let renders = h.borrow().renders;
        assert!(host.render_frames().is_empty());
        assert!(!h.borrow().pending_frame && h.borrow().renders == renders);
    }
}

// QM1g real backend (feature `media`): a frame comes out of libmpv's SW render at the
// requested size. SKIPS without libmpv or ffmpeg.
#[cfg(feature = "media")]
mod qm1g_real {
    use super::*;
    use crate::media_el::*;
    use crate::media_net::MediaMode;
    use crate::media_video::VideoTarget;

    #[test]
    fn mpv_renders_a_frame_at_the_target_size_and_skips_when_hidden() {
        if !crate::media_mpv::libmpv_available() {
            eprintln!("SKIP: libmpv.so.2 not available");
            return;
        }
        let dir = std::env::temp_dir().join(format!("qm1g-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("c.mp4");
        let ok = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=25", "-t", "2", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-movflags", "+faststart"])
            .arg(&p)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        let Some(file) = ok.then(|| std::fs::read(&p).ok()).flatten() else {
            eprintln!("SKIP: ffmpeg could not make a test clip");
            return;
        };
        let w = qm1e_wire(Qm1eCfg { file, ..Default::default() });
        let backend = crate::media_mpv::MpvBackend::new(BackendCtx { socket: Some(w.path.clone()), policy: Default::default() });
        let mut c = MediaCore::new(Box::new(backend));
        c.muted = true;
        c.set_target(Some(VideoTarget { w: 160, h: 120, fit: crate::css::ObjectFit::Contain }));
        c.load(vec![Candidate { url: "https://cdn.test/c.mp4".into(), mime: String::new() }], true, MediaMode::NoCors, "https://cdn.test/".into());
        let mut now = 0.0;
        let mut got = false;
        for _ in 0..300 {
            now += 20.0;
            c.pump(now, std::time::Duration::from_millis(20));
            c.take_outs();
            if c.frame_pending() && c.render_frame() {
                got = true;
                break;
            }
        }
        assert!(got, "no frame rendered");
        let f = c.last_frame.clone().unwrap();
        assert_eq!((f.w, f.h), (160, 120));
        assert!(f.px.iter().any(|&p| p & 0x00FF_FFFF != 0), "all black");
        assert!(f.px.iter().all(|&p| p >> 24 == 0), "top byte must stay clear");
        // hidden: no target, frames are consumed without a render
        c.set_target(None);
        c.play(1);
        for _ in 0..40 {
            now += 20.0;
            c.pump(now, std::time::Duration::from_millis(20));
            c.take_outs();
            if c.frame_pending() {
                assert!(!c.render_frame());
            }
        }
        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

// ---------- QM1k1n5: a finished page request is not queued behind a busy timer chain ----------

/// The event loop's turn is `run_due_slice` (bounded) and then whatever is queued, so a
/// `JsNet` that arrives while a chain of busy timers is due is delivered after at most one
/// slice, not after the chain: the response is seen before most of the chain has run, and
/// before a page-side timeout (the YouTube SABR abort) that became due after it arrived.
#[test]
fn qm1k1n5_response_arriving_during_a_busy_timer_chain_is_delivered_between_slices() {
    let fake = QfFake::new(vec![("/seg", qf7_ok("media", &[]))]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    let go = qf7_go("https://page.test/seg", "").replace("console.log('ok', r.status, r.type, t)", "console.log('ok ' + ran + ' ' + timedOut)");
    engine.eval(
        "t",
        &format!(
            "window.__spin = function(ms){{ var e = Date.now() + ms; while (Date.now() < e); }};\
             var ran = 0, timedOut = false;\
             for (var i = 0; i < 30; i++) setTimeout(function(){{ __spin(8); ran++; }}, 0);\
             setTimeout(function(){{ timedOut = true; }}, 40);\
             {go}"
        ),
    );
    // The page's request goes out; the "network thread" finishes it at once.
    let reqs = engine.take_requests();
    assert_eq!(reqs.net.len(), 1);
    let id = reqs.net[0].id;
    let done = fake.execute(&reqs.net[0]);
    std::thread::sleep(std::time::Duration::from_millis(5));
    // Loop turns: one 16 ms slice of the (now due) chain, then the queued JsNet.
    assert!(engine.run_due_slice(Some(std::time::Duration::from_millis(16))));
    engine.deliver_net(id, done);
    let l = log.borrow().clone();
    let ok = l.iter().find(|x| x.starts_with("log:ok ")).unwrap_or_else(|| panic!("not delivered: {l:?}"));
    let parts: Vec<&str> = ok.split(' ').collect();
    let ran: usize = parts[1].parse().unwrap();
    assert!(ran <= 4, "delivered after one slice, not after the 30-timer chain (ran {ran})");
    assert_eq!(parts[2], "false", "the 40 ms page timeout must not win against a response that arrived first");
}
