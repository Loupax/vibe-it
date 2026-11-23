# wire2

A local forward proxy whose traffic streams through a chain of external
filters, each a small HTTP server on its own Unix socket. It decrypts TLS only
for hosts a connected filter asks to see in plaintext; everything else stays an
opaque tunnel.

Status: **active (2026-11-23), no code yet.** The design is [`SPEC.md`](SPEC.md)
(the user's "Scriptable Filtering Proxy - Feature Spec"); the work list is `TODO.txt`.

wire2 is separate from [`wire`](../wire/) (wire 1), which stays the HTTP
client that scratchy and the web/ spike use.
