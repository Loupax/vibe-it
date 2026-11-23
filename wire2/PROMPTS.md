# wire2 — prompt log

Append-only log of the exact prompt that produced each change, in order.
The design discussion that led here is in ../web/PROMPTS.md (engine discussion).

---

## Plan (2026-11-23, TODO only)

> Here's the todo I want you to create: 1. save the current state of wire in it's own branch/tag. We start Wire 2 which is going to be a proxy from the beginning. No browser semantics, only stream data between server and browser. Again, should not buffer unless necessary. We should enable scripting later, maybe using an embedded language. Leave that for later, just keep in mind this is something we'll have to do.

> I think we are ready. Add a TODO for v2 scratchy to use this document for its rewrite

> Create a PR for adding the relevant TODO items for our new plan

> You know what, let's not over complicate things. Wire 2 should be on its own folder, wire2

Result: wire2/ with README.md, TODO.txt (W2-SPEC pointing at the spec, W2-1
plain forwarding proxy, W2-2 TLS interception, W2-3 WebSockets in decrypted
tunnels, W2-F filters per the spec) and this file. wire/ is unchanged. The
`wire-v1` branch made earlier for the in-place plan is no longer needed.

---

## Spec copied, wire2 active (2026-11-23)

> 1. Park the wire tasks in general. We only work on wire2 now. [...] 4. Yes, copy the spec. wire2 is active

Result: wire2/SPEC.md (the user's design doc, copied by the coordinator; its one
diagram redrawn as ASCII). W2-SPEC done; wire/ parked.
