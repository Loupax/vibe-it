# web

A keyboard-driven browser built on Chromium (Blink) through CEF, meant to be a
daily driver. It keeps the vibe-it networking contract: every request goes through
[`wire`](../wire/), and the browser itself cannot reach the network.

Status: **planning only, no code yet.** The first work is a throwaway spike
(TODO.txt) that answers one question: can a CEF browser be as complete as stock
Chrome while wire stays the only process that talks to the network?

`web` succeeds [`scratchy`](../scratchy/), which is paused (see
`../scratchy/DECISIONS.md`). scratchy's own engine (HTML/CSS/layout/paint/V8
bindings) is not carried over; its browser shell (keys, command line, history,
stash, completion) is ported after the spike says go.

- `DECISIONS.md`: what has been decided and why.
- `TODO.txt`: the spike steps, each with a check that ends it.
- `PROMPTS.md`: the exact prompts that produced this project, in order.
