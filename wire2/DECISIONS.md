# wire2 — decisions

Newest first. Each entry is a decision the user made, with the date.

## 2026-11-23 wire2's defaults are kept

User ("they all look good right now"): the CA directory stays `$XDG_DATA_HOME/proxy` as the spec says
(W2-2d); access-log byte counts include request/response heads (W2-1c); the filter binary is `wire2-pass`,
the unit `wire2.service`; a filter may change a request but not its destination; plain http also passes
tunnel-layer filters (W2-F's calls).

## 2026-11-23 wire2 is active; wire is parked

User: "Park the wire tasks in general. We only work on wire2 now." and "Yes, copy the spec.
wire2 is active". SPEC.md is the design and wins over TODO.txt where they differ. wire/ (wire 1)
gets no new work; it keeps running as it is for scratchy and the user's own setup.
