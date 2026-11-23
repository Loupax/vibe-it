# wire2

A local forward proxy whose traffic streams through a chain of external
filters, each a small HTTP server on its own Unix socket. It decrypts TLS only
for hosts a connected filter asks to see in plaintext; everything else stays an
opaque tunnel.

Status: **active (2026-11-23).** The design is [`SPEC.md`](SPEC.md)
(the user's "Scriptable Filtering Proxy - Feature Spec"); the work list is `TODO.txt`;
implementation notes and measurements are in [`NOTES.md`](NOTES.md).

Step W2-1 (plain forwarding proxy, no filters, no decryption) is the code so far:

```
cargo build --release
target/release/wire2 --listen 127.0.0.1:8080          # browser proxy: 127.0.0.1:8080 for HTTP and HTTPS
curl -x http://127.0.0.1:8080 https://example.com/    # CONNECT tunnel, nothing decrypted
kill -USR1 <pid>                                      # dump open flows to the log
```

Flags: `--listen host:port|unix:<path>`, `--allow-remote`, `--dial-timeout`, `--log-level`
(`wire2 --help`). Tests: `cargo test` (the 2 GB transfer tests take a few seconds in release);
measurements: `cargo test --release --test w2_1_perf -- --ignored --nocapture --test-threads 1`.

wire2 is separate from [`wire`](../wire/) (wire 1), which stays the HTTP
client that scratchy and the web/ spike use.
