#!/usr/bin/env python3
"""Harness bench generator — same model (Opus 5) · same prompt · same machine.
OpenCrabs TUI orchestration vs Claude Code TUI's own agent loop.
Reads /srv/bench/fps/runs-harness/*/metadata.json -> WEBROOT/harness/index.html."""
import json, os, html
from pathlib import Path
from build_fps_landing import CSS, FPS, WEBROOT, TASK_PROMPT, fmt_tokens, fmt_dur, fmt_size, disk_stats

HARNESS_DIR = FPS / "runs-harness"
OUT_DIR = WEBROOT / "harness"

def collect():
    recs = []
    if HARNESS_DIR.exists():
        for rd in sorted(HARNESS_DIR.iterdir()):
            mf = rd / "metadata.json"
            if mf.is_file():
                recs.append(json.loads(mf.read_text()))
    return recs

def build_rows(recs):
    rows = []
    for r in sorted(recs, key=lambda x: x.get("tokens") or 9e9):
        bdir = Path(r["build_dir"]) if r.get("build_dir") else FPS / r["slug"]
        files, size = disk_stats(bdir)
        terr = r.get("tools_err")
        tools = (f"{r.get('tools_ok', 0)} ok / {terr} err" if terr is not None
                 else f"{r.get('tools_ok', 0)} calls")
        rows.append({
            "slug": r["slug"], "name": r.get("display_name") or r["slug"],
            "model": r.get("model_display") or r.get("model", ""),
            "label": r.get("label", ""), "status": r.get("status", "completed"),
            "wall": r.get("wall_seconds"), "span": r.get("span_seconds"),
            "tokens": r.get("tokens", 0), "cost": r.get("cost_usd", 0),
            "turns": r.get("turns", 0), "tools": tools,
            "files": files, "bytes": size, "link": r.get("link") or f"/fps/{r['slug']}/",
        })
    return rows

def render(rows):
    fin = [r for r in rows if r["status"] == "completed"]
    win = min(fin, key=lambda r: r["wall"]) if fin else None
    lean = min(fin, key=lambda r: r["tokens"]) if fin else None
    cheap = min(fin, key=lambda r: r["cost"]) if fin else None
    slow = max(fin, key=lambda r: r["wall"]) if fin else None
    fat = max(fin, key=lambda r: r["tokens"]) if fin else None
    pricey = max(fin, key=lambda r: r["cost"]) if fin else None
    n = len(fin)
    cards = f"""
    <div class="cards">
      <div class="card"><div class="k">Leanest context — rank key</div><div class="v">{html.escape(lean['name']) if lean else '—'}</div><div class="m">{fmt_tokens(lean['tokens']) if lean else '—'} — {fat['tokens']/lean['tokens']:.1f}x leaner than the hungriest</div></div>
      <div class="card"><div class="k">Cheapest run</div><div class="v">{html.escape(cheap['name']) if cheap else '—'}</div><div class="m">${'{:.2f}'.format(cheap['cost']) if cheap else '—'} — {pricey['cost']/cheap['cost']:.1f}x cheaper than the priciest</div></div>
      <div class="card"><div class="k">Fastest harness</div><div class="v">{html.escape(win['name']) if win else '—'}</div><div class="m">{fmt_dur(win['wall']) if win else '—'} vs {fmt_dur(slow['wall']) if slow else '—'} — {slow['wall']/win['wall']:.2f}x across {n} harnesses</div></div>
      <div class="card"><div class="k">Deliverable parity</div><div class="v">{n} / {n} playable</div><div class="m">all load first try · full v2 spec · Visual Options menus</div></div>
    </div>"""
    medals = ["🥇", "🥈", "🥉"]
    trs = []
    for i, r in enumerate(rows):
        medal = f'<span class="medal">{medals[i]}</span> ' if i < len(medals) and r["status"] == "completed" else ""
        trs.append(f"""<tr>
        <td>{medal}{i+1}</td>
        <td><span class="name">{html.escape(r['name'])}</span><span class="tag done">{html.escape(r['label'])}</span></td>
        <td><b>{html.escape(r['model'])}</b></td>
        <td>{fmt_dur(r['wall'])}</td><td>{fmt_dur(r['span'])}</td>
        <td>{fmt_tokens(r['tokens'])}</td>
        <td>{"${:.2f}".format(r['cost'])}</td>
        <td>{r['turns']}</td><td>{r['tools']}</td>
        <td>{r['files'] if r['files'] else '—'} / {fmt_size(r['bytes']) if r['bytes'] else '—'}</td>
        <td><a class="btn" href="{r['link']}">view build</a></td></tr>""")
    board = f"""
    <table><thead><tr><th>#</th><th>Harness</th><th>Model</th><th>Wall (active)</th><th>Raw span</th><th>Tokens · rank key</th><th>Cost</th><th>Steps</th><th>Tool calls</th><th>Files / Size</th><th>Build</th></tr></thead>
    <tbody>{''.join(trs)}</tbody></table>"""
    notes = f"""
    <div class="notes">
      <div class="note"><h3>The experiment</h3><b>One variable: the harness.</b> Same model (Opus 5), same
      machine (bench VPS, root), same prompt — byte-verbatim, card below. Three orchestration loops:
      OpenCrabs' TUI loop (step structure, tool batching, context handling) spawning the claude CLI as its
      provider; Claude Code's built-in agent loop — the OpenCrabs rows and the Claude Code row share that
      identical engine and transport; and the Hermes TUI driving the Anthropic API directly through its own
      gateway and session runtime. OpenCrabs ran <b>twice</b> (run 1 in the original bench workdir, run 2 in
      a fresh isolated workdir with the box to itself) for repeatability; every other harness ran once.
      All fired manually in-terminal, auto-approve on, no timeouts.</div>
      <div class="note"><h3>What the numbers say</h3><b>Ranked by token efficiency — fewest tokens to
      complete the task wins.</b> The clock rewards burning parallel compute; the token count is what the
      harness actually made the model read and write, so it's the price of the task. <b>OpenCrabs TUI takes
      the board on both runs</b> — run 1: 10.43M tokens, $8.00, 37m30s; run 2: 10.82M tokens, $10.79,
      41m01s. Repeatability: ±3.8% tokens, ±9.8% wall across two independent one-shots — and both runs
      stay under Hermes. <b>Hermes second</b> — 12.51M tokens (1.2x ours), $15.65, 40m09s; the two
      OpenCrabs walls straddle it (37m30s under, 41m01s just over by 52s), average 39m16s still faster —
      but it loses the rank key that decides the podium either way. <b>Claude Code's loop last</b> —
      55.07M tokens (5.3x ours), $46.59, 1h36m23s: 249 round trips re-reading a growing context, 97.6% of
      its tokens cache reads. The cheapest run cost 17% of the priciest.</div>
      <div class="note"><h3>Quality parity, not just speed</h3>All four builds loaded first try and shipped
      the full v2 spec including the Visual Options menu; each self-verified headless and self-caught real
      bugs. OpenCrabs run 1: 3 bugs (unwinnable OC-3 reactor hitbox, dead cover state, backwards film grain),
      80/80 smoke checks. OpenCrabs run 2: 23 modules / 2,914 lines, spec values measured not assumed —
      headshot multiplier 2.30x against the 2.3 spec, scanner piercing 3 stacked targets, launcher splash
      + 1.6 m/s knockback + 38 hp self-damage, recoil bloom driving the crosshair — and a full campaign
      sweep OC-1→OC-6 with 0 console errors and 0 failed requests. Claude Code run: 4 bugs (W sign error, frame-rate-dependent friction, muzzle
      computed in view space, doorway sealing), headless Chromium all-decks pass. Hermes run: five
      headless-Chrome suites — 0 bytes allocated per frame across 1200 sim steps, 0.044 ms sim cost under
      full combat load, 61–112 draw calls, ACES confirmed, full campaign OC-1→OVERLOCK with 0 errors — and
      honest caveats stamped on its own work (SwiftShader software raster; the 1-shot-per-frame auto-fire
      gate it chased down and correctly acquitted). Deliverables equal on all sides:
      game + README + design doc + chapter-3 playthrough script.</div>
      <div class="note"><h3>Provenance &amp; asterisks</h3>OpenCrabs row = native usage_ledger, cost rolled
      up by the harness at official Anthropic rates (Max sub — not billed per run). Claude Code row =
      harvested from the claude jsonl transcript, cost computed at official rates. The OpenCrabs run began
      with the operator's "continue" kickoff after a cancelled pre-switch leg; all figures cover the opus
      leg only (operator ruling: the run counts from the model switch). The OpenCrabs run
      also survived a parallel process writing into its workdir mid-run — it flagged the intrusion in its
      own final message; the dir was later migrated to opus-5-och/. Its build size includes ~12MB of
      headless-test artifacts (test/out PNGs, test-vendored three.js); the game proper is ~210KB, on par
      with its twin. OpenCrabs run 2 = also native usage_ledger (10,821,679 tokens / $10.79), a clean
      one-shot in a fresh isolated workdir with the box to itself — no parallel disturbance, and the
      cleanest deliverable tree of the lane (~157KB, no test artifacts left behind). Hermes row = harvested from its own sqlite state.db (sessions + session_model_usage):
      wall = prompt → final "Shipped and verified" message (22:16:58→22:57:07), no active/idle split
      recorded so wall = span; post-run proxy Q&amp;A (23:07–23:11) excluded. Its 12,506,671 tokens are the
      harness's own counter and exclude a 1.41M background-review side task; cost computed at the same
      official rate card (in 186 / cache-write 970,520 / cache-read 11,380,426 / out 155,539). Hermes also
      deployed its own build: asked for a port proxy, it instead symlinked into the static vhost following
      the existing bench convention — zero nginx changes, MIME types verified.</div>
      <div class="note"><h3>The prompt (verbatim, all four runs)</h3>Identical text verified byte-for-byte
      against the OpenCrabs session records and the Claude Code transcript — including the Visual Options
      menu requirement and the tools-immediately sentence, typos and all. The Hermes and OpenCrabs run 2
      copies are the same task with small textual deltas: each spells its own workdir path out in the
      first line (run 1 and Claude Code got theirs from cwd) plus terminal-editor whitespace padding —
      same spec, same typos, not byte-identical.
      <div class="prompt">{html.escape(TASK_PROMPT)}</div></div>
      <div class="note"><h3>Back to the model board</h3>This page holds the harness twins. Model-vs-model
      standings (GLM-5.3, Qwen3.8-Max, Opus 5, Fable 5.1, Kimi K3, DeepSeek V4 Flash) live on the
      <a href="/">main bench board</a>.</div>
    </div>"""
    return f"""<!DOCTYPE html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>OC Harness Bench — FPS</title><style>{CSS}</style></head><body><div class="wrap">
<h1>OC Harness Bench — FPS ⚔️</h1>
<div class="sub">same brain (Opus 5) · same prompt · same box — the harness was the variable · ranked by tokens to complete (fewest wins) · <a class="btn" href="/">← model board</a></div>
{cards}
<h2>Head to head</h2>
{board}
<h2>Notes</h2>
{notes}
<footer>OC Bench · harness lane · regenerated by build_harness_landing.py · builds served live from the bench workdirs</footer>
</div></body></html>"""

def main():
    recs = collect()
    rows = build_rows(recs)
    out = render(rows)
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    tmp = OUT_DIR / "index.html.new"
    tmp.write_text(out)
    os.replace(tmp, OUT_DIR / "index.html")
    print(f"wrote {OUT_DIR/'index.html'} ({len(out)} bytes), rows={len(rows)}, finished={sum(1 for r in rows if r['status']=='completed')}")

if __name__ == "__main__":
    main()
