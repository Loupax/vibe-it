#!/usr/bin/env python3
"""QINC1k: one table row per tools/incr-gate/run.sh run.

Usage: python3 -I tools/incr-gate/summarize.py OUT_DIR [TAG_PREFIX ...]

Window runs (<tag>.stats = SCRATCHY_FRAME_STATS lines, <tag>.log, <tag>.rss):
  builds n/ms     App frame work on the main thread (sum of `builds`)
  wrk             shaping-worker ms
  forced n/ms     script forced layouts (sum of `forced`)
  p95             median over the seconds WITH input of the per-second input-to-present p95 (ms)
  hov             hover latency: median over the seconds with pointer MOVES of the per-second p95
  gPS n/med       getPlayerSize forced layouts (frame stats top-3 callers per second): count, median
                  over seconds of the per-second mean ms per read
  worst           longest callback after t >= 15 s (ms, kind) = the preview-start promise job on ytr
  vid             video frames painted (> 0 = the "active playback" regime on ytw)
  rss             VmRSS at the end (MB)
  mm / pan        `incr-check: MISMATCH` lines / panics in the log
  frames c/m/w    incr Current / Main / Worker App frames; f/r = forced layouts through the engine /
                  restarts; root = relayouts walked to the root; wf = worker fallbacks by reason
Headless runs (<tag>.json = --perf-report): forced n/ms, incr counters, rss_bytes, mismatches.
"""
import json
import os
import re
import statistics
import sys


def med(v):
    return statistics.median(v) if v else 0.0


def meta(path):
    m = {}
    try:
        for line in open(path):
            for part in line.split():
                if '=' in part:
                    k, v = part.split('=', 1)
                    m.setdefault(k, v)
            if line.startswith('uptime') or 'uptime_' in line:
                for k in ('uptime_start', 'uptime_locked', 'uptime_end'):
                    i = line.find(k + '=')
                    if i >= 0:
                        m[k] = line[i + len(k) + 1:].strip().split(' ')[0].rstrip(',')
    except OSError:
        pass
    return m


def log_facts(path):
    mm = pan = 0
    incr = None
    diags = []
    try:
        for line in open(path, errors='replace'):
            if 'incr-check: MISMATCH' in line:
                mm += 1
                if len(diags) < 5:
                    diags.append(line.strip()[:300])
            if 'panicked' in line:
                pan += 1
            if line.startswith('scratchy: incr: '):
                try:
                    incr = json.loads(line[len('scratchy: incr: '):])
                except ValueError:
                    pass
    except OSError:
        pass
    return mm, pan, incr, diags


def incr_cols(i):
    if not i:
        return '-'
    wf = ','.join(f'{k}:{v}' for k, v in i.get('worker_fallbacks', {}).items())
    return (f"{i.get('frames_current', 0)}/{i.get('frames_main', 0)}/{i.get('frames_worker', 0)} "
            f"f{i.get('forced_engine', 0)}/r{i.get('forced_restarts', 0)} root{i.get('layout_root', 0)} "
            f"full{i.get('layout_full', 0)} rs{i.get('restyle', {}).get('ms', 0):.0f} "
            f"lay{i.get('layout_ms', 0):.0f}+{i.get('layout_full_ms', 0):.0f} box{i.get('box_ms', 0):.0f}"
            + (f' wf[{wf}]' if wf else ''))


def window_row(out, tag):
    lines = []
    try:
        for line in open(os.path.join(out, tag + '.stats')):
            try:
                lines.append(json.loads(line))
            except ValueError:
                pass
    except OSError:
        return None
    m = meta(os.path.join(out, tag + '.meta'))
    bn = sum(x['builds']['n'] for x in lines)
    bms = sum(x['builds']['ms'] for x in lines)
    wrk = sum(x['shape']['worker_ms'] for x in lines)
    fn = sum(x['forced']['n'] for x in lines)
    fms = sum(x['forced']['ms'] for x in lines)
    inp = [x for x in lines if sum(x['input'][k] for k in ('move', 'button', 'key', 'wheel')) > 0]
    mv = [x for x in lines if x['input']['move'] > 0]
    p95 = med([x['input']['lat_p95_ms'] for x in inp])
    hov = med([x['input']['lat_p95_ms'] for x in mv])
    gps_n = 0
    gps_means = []
    for x in lines:
        for c in x['forced']['by_caller']:
            if 'getPlayerSize' in c['caller']:
                gps_n += c['n']
                if c['n']:
                    gps_means.append(c['ms'] / c['n'])
    worst = (0.0, '')
    for x in lines:
        lg = x.get('longest')
        if lg and x['t'] >= 15 and lg['ms'] > worst[0]:
            worst = (lg['ms'], lg['kind'])
    vid = sum(v['painted'] for x in lines for v in x['video']['v'])
    rss = '-'
    try:
        for line in open(os.path.join(out, tag + '.rss')):
            if line.startswith('VmRSS'):
                rss = str(int(line.split()[1]) // 1024)
    except OSError:
        pass
    mm, pan, incr, diags = log_facts(os.path.join(out, tag + '.log'))
    row = (f"| {tag} | {m.get('mode', '?')} | {bn} / {bms:.0f} | {wrk:.0f} | {fn} / {fms:.0f} | {p95:.0f} | {hov:.0f} | "
           f"{gps_n} / {med(gps_means):.1f} | {worst[0]:.0f} {worst[1]} | {vid} | {rss} | {mm} | {pan} | "
           f"{m.get('quit', '?')} | {m.get('uptime_locked', '?')} | {incr_cols(incr)} |")
    return row, diags


def headless_row(out, tag):
    try:
        d = json.load(open(os.path.join(out, tag + '.json')))
    except (OSError, ValueError):
        return None
    m = meta(os.path.join(out, tag + '.meta'))
    fl = d.get('forced_layout', {})
    i = d.get('incr', {})
    mm, pan, _, diags = log_facts(os.path.join(out, tag + '.log'))
    rss = d.get('rss_bytes', 0) // (1 << 20)
    worst = max((x.get('raw_ms', 0) for x in d.get('longest', [])), default=0)
    row = (f"| {tag} | {m.get('mode', '?')} | {fl.get('count', 0)} / {fl.get('total_ms', 0):.0f} | "
           f"{fl.get('max_ms', 0):.0f} | {fl.get('phases', {}).get('incr_shadow', {}).get('total_ms', 0):.0f} | "
           f"{worst:.0f} | {rss} | {i.get('check_mismatch', 0)} / {mm} | {pan} | {d.get('outcome')} | "
           f"{m.get('uptime_locked', '?')} | {incr_cols(i)} |")
    return row, diags


def main():
    out = sys.argv[1]
    prefixes = sys.argv[2:]
    tags = sorted({f.rsplit('.', 1)[0] for f in os.listdir(out) if f.endswith('.meta')})
    if prefixes:
        tags = [t for t in tags if any(t.startswith(p) for p in prefixes)]
    win, hl, diag = [], [], []
    for t in tags:
        k = meta(os.path.join(out, t + '.meta')).get('kind')
        r = window_row(out, t) if k == 'window' else headless_row(out, t)
        if not r:
            continue
        (win if k == 'window' else hl).append(r[0])
        diag += [f'{t}: {d}' for d in r[1]]
    if win:
        print('| run | mode | builds n / ms | wrk ms | forced n / ms | p95 | hov p95 | gPS n / med ms | worst job | vid | rss MB | mm | pan | quit | load | incr c/m/w frames, forced/restarts, ... |')
        print('|' + '---|' * 16)
        print('\n'.join(win))
    if hl:
        print()
        print('| run | mode | forced n / ms | max ms | incr_shadow ms | worst cb ms | rss MB | mm (counter / lines) | pan | outcome | load | incr |')
        print('|' + '---|' * 12)
        print('\n'.join(hl))
    if diag:
        print()
        print('MISMATCH diags (first 5 per run):')
        print('\n'.join(diag))


if __name__ == '__main__':
    main()
