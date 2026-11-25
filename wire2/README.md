# wire2

A local forward proxy whose traffic streams through a chain of external
filters, each a small HTTP server on its own Unix socket. It decrypts TLS only
for hosts a connected filter asks to see in plaintext; everything else stays an
opaque tunnel.

Status: **active (2026-11-23).** The design is [`SPEC.md`](SPEC.md)
(the user's "Scriptable Filtering Proxy - Feature Spec"); the work list is `TODO.txt`;
implementation notes and measurements are in [`NOTES.md`](NOTES.md).

Steps W2-1 (plain forwarding), W2-2 (TLS interception), W2-3 (WebSockets) and W2-F (the filter chain
and the reference filter `pass`) are done. Two binaries: `wire2` (the proxy) and `wire2-pass` (the
filter `pass`: tunnel layer, all hosts, fail open, forwards everything unchanged; named so it does not
shadow the `pass` password manager).

Run it with `pass` by hand:

```
cargo build --release
wire2-pass --socket $XDG_RUNTIME_DIR/pass.sock &                    # the filter (mode 0600 socket)
wire2 --listen 127.0.0.1:8080 --filter unix:$XDG_RUNTIME_DIR/pass.sock
curl -x http://127.0.0.1:8080 https://example.com/    # CONNECT tunnel through pass, nothing decrypted
kill -USR1 <wire2 pid>                                # filter states and open flows to the log
```

(the binaries are in `target/release/`; `coordinator/ci_then_push.sh --install wire2` puts both in
`~/.cargo/bin`). The proxy's hub socket defaults to `$XDG_RUNTIME_DIR/proxy-hub.sock`. Or with systemd
user units (pass is socket-activated, so the proxy's first connection starts it):

```
cp systemd/pass.socket systemd/pass.service systemd/wire2.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now pass.socket wire2.service
journalctl --user -u wire2 -f                         # one access line per flow, with chain=pass
```

Point the browser's HTTP and HTTPS proxy at 127.0.0.1:8080. With only `pass` connected nothing is
decrypted and no CA exists. Filters are chained in `--filter` order; one that is down refuses its hosts
with 503 if it fails closed (before its first handshake: every host) and is skipped if it fails open.
Once an http-layer filter claims hosts, the CA is created in `--ca-dir` (default `$XDG_DATA_HOME/proxy`);
`ca.pem` there is what the browser imports, and `--no-decrypt GLOB` keeps hosts opaque whatever a filter
claims.

Flags: `--listen host:port|unix:<path>`, `--allow-remote`, `--filter unix:<path>`, `--hub <path>`,
`--connect-timeout`, `--stage-timeout`, `--filter-check D` (default 2s, re-handshakes an up filter so a restart or changed capabilities are noticed without traffic; an addition beyond SPEC.md, see NOTES.md "W2-F1"), `--pool-idle D` (30s), `--pool-per-host N` (6), `--pool-total N` (64) (the shared upstream connection pool; additions beyond SPEC.md, see NOTES.md "W2-2e"), `--dial-timeout`, `--log-level`, `--no-decrypt GLOB`,
`--ca-dir DIR` (`wire2 --help`, `wire2-pass --help`). Writing a filter: SPEC.md "Filter protocol";
`src/pass.rs` is the smallest complete one. Tests: `cargo test --release` (the 2 GB transfer tests take
a few seconds in release; decrypted tests use the test-only pass-decrypt filter and the TEST-ONLY
`--test-resolve`, see NOTES.md); measurements: `cargo test --release --test w2_1_perf` / `w2_2_perf` /
`w2_f_perf` `-- --ignored --nocapture --test-threads 1`.

wire2 is separate from [`wire`](../archived/wire/) (wire 1), which stays the HTTP
client that scratchy and the web/ spike use.
