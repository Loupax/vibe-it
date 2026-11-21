# TODO.txt triage — 2026-11-19 ~20:45 (session 2), 218 open items

Governing rule (user, in TODO.txt): performance items outrank everything; no backlog item starts while a READY perf item
is unassigned; QPROF1 closes the perf phase. So "ready" below = startable NOW under that rule.

## READY now (perf queue + scheduled exceptions)
Wave 1 (WIP): QM1k2 research (Opus) | QYTC1 (Opus) | QMEM3e #369 quiet check (Sonnet) | QOBS6 (Sonnet) | QY53o1 (Sonnet) | QAN1h2 (Sonnet)
Next, by area (never two in one area at once):
- app.rs / perf.rs (after QOBS6): QOBS5c [S], QY53m [S], QY53n1 [S], Q33b2 [S] (innerWidth/Height = window size) — one at a time
- js_dom.rs (after QY53o1): QSTK1b [S], QY47b [S]
- css / style matching (after QAN1h2): QAN1h3 [S], QY53f1d [M, Opus design]
- animation.rs: QY53i3 [S]   - media: QM1g2a [M, Opus] (split (a) out of QM1g2)
- memory: QMEM3b [S, measure], QMEM3d [S], QMEM3f [S, Opus measure]   - optional: QY53l3 [S]

## BLOCKED by another item
QM1k1n5 <- QOBS6 (#370 merging) | QYTC1a, QYTC1b <- QY53o1 (js_dom.rs hook) | QYTC1c <- QAN1h2 (css area) | QY53m2 <- QY53m | QY53e2 <- Q33b2 | QY53n2 <- QY53n1; QY53n3, QY53n5 <- QY53n2; QY53n4 (P4, later)
QY53o2 <- QY53o1 (only if > ~3 s left) | QY53f2, QY53f3, QY53o3c <- QPROF1 / fresh measurement
QY60, QY54, QY33, QOBS9 <- rest of perf queue + quiet machine | QPROF1 <- whole perf queue | QYS1 <- QPROF1
QM1k2 fixes <- QM1k2 research + user choice
Everything else (backlog: QUA1, QV8-3c1, QM1g2d1, QM1g2e, QY49-D, QCFG1->QTB1, QCI2, QM1a, QM1k1*/QM1k-u/QM1k5,
QWK0..QWK-m chain, QSD2b/5/6..12, QF*, QFF1a..c, QAN1d..g/i, QJ*, QW*, QY*, QA*, Q26, Q30*, Q33*, Q41b..Q75, QSB1, ...)
<- the perf queue + QPROF1 (user rule). Internal chains noted in each entry (e.g. QWK1 <- QWK0, QSD7 <- QSD6, QFF1b/c <- QFF1a).
Parked by the user (not scheduled): QM1k3, QTB2, QAN1p, Q32, QM1k-r (dropped), QM1k-B (only if QM1k-AB fails).
History/detail-section checkboxes (untitled "Multiple floats...", "**Scrollbar drag...", "**Form elements**", engine choice,
...) are superseded by queue items (Q32, QSB1, Q30*, QV8) — not work items.

## NEEDS CLARIFICATION (asked 2026-11-19)
1. QV8-6 remove QuickJS (+ stop running the local engine-quickjs suite): TODO says YES, STATE said "user decides 2026-11-20".
2. QM1k-mouse: user's YouTube re-check (needs QCUR1 installed; new frame-stats filename).
3. Merge: rebase conflicts in append-only PROMPTS.md / REGRESSIONS.md: auto keep-both, or back to the agent?
4. QM1h (media helper process) only if D2 = separate process; D2 was decided in-process -> drop?
5. NEEDS HUMAN CHECK: QY20b (bbc tick glyphs) and QY15c (old.reddit) live re-runs.
