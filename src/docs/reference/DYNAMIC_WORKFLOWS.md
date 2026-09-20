# Dynamic Workflows in OpenCrabs

> Orchestrate many agents with scripts instead of chat turns. This guide shows every pattern users of other tools call "dynamic workflows", implemented entirely with OpenCrabs features that ship today.

## The core idea

A dynamic workflow moves **the plan out of the model's context window and into code**. Instead of one long conversation where the model decides turn-by-turn what to do next, you write a script where:

- each step spawns an agent as a plain function call,
- intermediate results live in **files**, not in a context window,
- filtering, branching, loops and retries are exact shell/code constructs that cost zero tokens,
- the whole thing is re-runnable, schedulable and resumable.

OpenCrabs supports this through its non-interactive entrypoint:

```bash
opencrabs run "<prompt>" --format json --auto-approve
```

| Flag | What it does |
|------|--------------|
| `run <PROMPT>` | Executes a single task non-interactively, then exits |
| `--format text\|json\|markdown` | Output format (`-f`, default `text`) |
| `--auto-approve` (alias `--yolo`) | Auto-approves tool executions inside the run |

One process = one agent = one task. Your script is the orchestrator.

---

## Recipe 1: Fan-out map (N parallel agents)

You have a directory of task files and want them all processed concurrently:

```bash
mkdir -p out

for t in tasks/*.md; do
  name=$(basename "$t" .md)
  opencrabs run "$(cat "$t")" --format json --auto-approve \
    > "out/${name}.json" &
done
wait   # barrier: block until all agents finish
```

With concurrency control (max 4 at a time), use GNU parallel:

```bash
ls tasks/*.md | parallel -j4 '
  n=$(basename {} .md)
  opencrabs run "$(cat {})" --format json --auto-approve > "out/${n}.json"
'
```

Each agent gets a **fresh context** — no cross-contamination between tasks, no compaction pressure, and one slow or failed task never blocks the others.

## Recipe 2: Structured outputs ("schema enforcement")

`--format json` gives you machine-parseable output. Combine with `jq` for validation-and-retry, which is what schema-enforced subagents do elsewhere:

```bash
run_validated() {
  local prompt="$1" out="$2" tries=0
  while (( tries < 3 )); do
    opencrabs run "$prompt" --format json --auto-approve > "$out"
    # accept only if output parses and has the fields we need
    if jq -e '.result and .summary' "$out" >/dev/null 2>&1; then
      return 0
    fi
    tries=$((tries + 1))
  done
  return 1
}
```

The contract lives in your script, checked mechanically — no trusting prose.

## Recipe 3: Pipelines (stages over items)

Stage 2 consumes stage 1's output. Keep intermediates on disk so every stage is independently debuggable and re-runnable:

```bash
# Stage 1: triage each issue
for i in issues/*.txt; do
  n=$(basename "$i" .txt)
  opencrabs run "Classify this issue as bug/feature/question, answer in JSON: $(cat "$i")" \
    -f json --auto-approve > "out/${n}.class.json" &
done
wait

# Stage 2: only bugs go to the fixer
for f in out/*.class.json; do
  jq -r 'select(.result == "bug") | input_filename' "$f"
done | while read -r cls; do
  n=$(basename "$cls" .class.json)
  opencrabs run "Fix the bug described in issues/${n}.txt. Run the tests." \
    --auto-approve > "out/${n}.fix.log" &
done
wait
```

Items flow through stages independently; the filesystem is your message bus.

## Recipe 4: Checkpoint and resume

Because state lives in files, resuming after a crash or interrupt is a two-line idiom — skip anything already done:

```bash
process() {
  local n="$1"; shift
  [ -s "out/${n}.json" ] && return 0        # already finished: skip
  opencrabs run "$*" --format json --auto-approve > "out/${n}.json"
}

for t in tasks/*.md; do
  process "$(basename "$t" .md)" "$(cat "$t")" &
done
wait
```

Re-running the script costs nothing for completed work and picks up exactly where it stopped.

## Recipe 5: Isolation (repo-mutating agents)

Agents that edit code should not share one working tree. Hand each its own worktree:

```bash
fix_one() {
  local n="$1"
  git worktree add "wt-${n}" -b "agent/${n}" main
  (
    cd "wt-${n}"
    opencrabs run "Implement task ${n} from ../tasks/${n}.md. Commit your work." \
      --auto-approve > "../out/${n}.log"
  )
  # merge or drop wt-N / agent-N later — your review gate
}
```

Merge branches only after human review — the script proposes, you dispose.

## Recipe 6: In-app orchestration (no shell needed)

Inside a running crab session, the same patterns exist as native tools:

- **Plan imports** — save a workflow as a JSON plan and execute it any time: dependency ordering, acceptance criteria, and `isolated: true` to force each task into a freshly spawned worker. See the [Plan JSON Specification](plans/plan-json-spec.md).
- **Subagents & teams** — `spawn_agent` / `wait_agent` / `send_input` / `resume_agent` for individual workers, `team_create` / `team_broadcast` for crews sharing a task list.
- **Skills & slash commands** — package a recurring workflow as `/my-flow` so humans trigger it by name.

Rule of thumb: **shell recipes** for high-fanout mechanical maps; **in-app plans** when you want the model supervising execution against acceptance criteria.

## Recipe 7: Time-based triggers

Two different tools, pick by semantics:

| Need | Tool | Behavior |
|------|------|----------|
| Nightly sweep, reminders, periodic audits | **cron job** | Fresh context every fire; survives restarts; process-independent |
| Self-checks while idle | **cron + HEARTBEAT.md** | A cron job reads the HEARTBEAT.md checklist on a schedule |
| Iterate on live context repeatedly | **chat session** | Accumulating context, session-bound |

A cron entry like `0 4 * * 1` delivering "run the audit workflow above" into a chat gives you scheduled dynamic workflows with zero extra infrastructure.

## Recipe 8: Scale across machines (A2A)

Fan-out doesn't have to be local. The A2A gateway speaks JSON-RPC between crabs on **different VPS machines**: designate two boxes as workers, and your orchestrator script dispatches remotely instead of spawning locally. Same script shape, wider hardware budget. See the A2A Gateway skill reference for setup.

---

## How this compares

For readers coming from other tools' "dynamic workflows":

| Capability | Scripted OpenCrabs |
|------------|--------------------|
| Agents as function calls | ✅ `opencrabs run` per step |
| Plan held by code, zero-token control flow | ✅ shell/jq constructs |
| Structured outputs | ✅ `-f json` + mechanical validation |
| Parallel fan-out with barrier | ✅ background jobs + `wait` / `parallel -jN` |
| Mid-run resume | ✅ skip-if-done checkpointing on files |
| Per-agent isolation | ✅ git worktrees |
| Saved, named workflows | ✅ plan JSON imports + skills/slash commands |
| Scheduled runs surviving restarts | ✅ native cron |
| Cross-machine distribution | ✅ A2A JSON-RPC |

## Honest gotchas

- **Boot cost per spawn.** Each `opencrabs run` loads config, brain files and DB (seconds). At modest fan-out it's irrelevant; for hundreds of tiny steps, batch related work into fewer, bigger prompts.
- **No built-in mid-run dashboard.** Observability = your `out/` directory plus logs. The flip side: state you can `ls`.
- **`--auto-approve` is powerful.** Tools execute without confirmation — scope each agent to its own worktree/directory so a runaway can't touch shared state.
