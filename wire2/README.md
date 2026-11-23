# wire2

A local forward proxy whose traffic streams through a chain of external
filters, each a small HTTP server on its own Unix socket. It decrypts TLS only
for hosts a connected filter asks to see in plaintext; everything else stays an
opaque tunnel.

Status: **planning only, no code yet.** See `TODO.txt`; the design is the
user's spec "Scriptable Filtering Proxy - Feature Spec", to be copied into
`SPEC.md` before the first code.

wire2 is separate from [`wire`](../wire/) (wire 1), which stays the HTTP
client that scratchy and the web/ spike use.
