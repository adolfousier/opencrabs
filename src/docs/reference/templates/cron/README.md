# Cron Job Templates

Copy-paste starting points for OpenCrabs scheduled jobs. The `.sh` files in
`scripts/` are generic, dependency-free (pure bash, no Python), and contain no
personal data. Adapt them to your task, drop them in `~/.opencrabs/scripts/`,
and point a cron job at them.

## How OpenCrabs cron actually runs

A few facts decide how you should author a job:

| Fact | Consequence |
|------|-------------|
| All jobs share **one** cron session, with a context-compaction boundary inserted **after every run** | A run starts with **empty context**. The model remembers nothing from prior runs. Durable state must live in a file, a DB, or the script's own output. Job B never sees Job A's history. |
| Tools are **auto-approved** inside a cron run | The model *can* call bash / http / any tool. An LLM-driven cron is a full agent loop, not a one-shot text reply. Failures are never a permission problem. |
| The model **re-reasons from scratch** every fire | Output drifts run to run. The more logic you bake into the prompt, the less reliable it gets. |

**Reliability is inversely proportional to how much reasoning lives in the
prompt.** The fix is the script-runner pattern below.

## The script-runner pattern (preferred)

Make the **script** deterministic and let the model be **only the trigger**:

```
Prompt: "Run ~/.opencrabs/scripts/health-check.sh and reply with its output."
```

You lose nothing (the script can call APIs, run `opencrabs` subcommands, hit
any CLI) and you gain:

- **Determinism** — same logic every fire, no prompt drift.
- **Testability** — run it by hand to debug, no waiting for the schedule.
- **Environment control** — the script sets its own `cd`, `PATH`, and env.

### The bash-in-cron gotcha

A cron run has no TTY, a different working directory, and a leaner `PATH` than
your interactive TUI. A command typed in the TUI can fail under cron purely
because of that thinner environment. The script-runner pattern sidesteps this:
every script here declares its own `PATH` / `cd` up top before doing any work.
Always use **absolute paths** to binaries and files inside cron scripts.

## Cron expression format

OpenCrabs uses a **5-field** expression interpreted in the job's timezone's
wall clock (DST-aware):

```
min  hour  day-of-month  month  day-of-week
```

Two footguns:

- **Day-of-week is `1-7 = Sun-Sat`** (1=Sunday, 7=Saturday; `0` is rejected).
  Prefer names: `Mon-Fri`, `Sun`, `Sat` are unambiguous.
- **`@daily` / `@hourly` macros do not work.** Write the explicit fields.

| You want | Expression |
|----------|------------|
| Every day at 09:00 | `0 9 * * *` |
| Weekdays at 08:30 | `30 8 * * Mon-Fri` |
| Every hour on the hour | `0 * * * *` |
| Every 15 minutes | `*/15 * * * *` |
| Sundays at 18:00 | `0 18 * * Sun` |
| First of the month, 00:00 | `0 0 1 * *` |

## Trigger-gated jobs

A job can carry a **pre-flight trigger**: at each fire time the scheduler first
runs a shell command, and only executes the prompt when a condition on that
command's result is met. This turns cron jobs from "runs no matter what" into
"runs when something actually happened" (a file appeared, a health check
failed, a log line matched).

Set it through the `cron_manage` tool at create or update time:

| Field | What it does |
|-------|--------------|
| `trigger_cmd` | Shell command run under `/bin/sh -c` before the prompt. 30s timeout; a hung trigger is killed, not orphaned. Either `prompt` or `trigger_cmd` must be set. |
| `trigger_on` | Fire condition (see below). Default `non_empty`. |
| `set_goal` | On fire, set the active goal in the destination session instead of a plain prompt turn. Requires `deliver_to` to target a session; refused for passive channel deliveries. |
| `goal_template` | Template for the goal/notification text. Interpolates `{output}`, `{stdout}`, `{stderr}`, `{exit_code}`. |

### `trigger_on` conditions

| Value | Fires when |
|-------|-----------|
| `non_empty` (default) | stdout or stderr contains any non-whitespace output |
| `exit_zero` | the trigger exits 0 |
| `exit_non_zero` | the trigger exits non-zero (health checks, `grep` misses) |
| `regex:PATTERN` or `re:PATTERN` | combined output matches the regular expression |
| `always` | the trigger finished, regardless of output or exit code |

### Semantics

- **Condition not met** → the run is short-circuited at 0 tokens and recorded
  as a skipped/success run. The prompt never executes, nothing is delivered.
- **Trigger command errors or times out** → the run is recorded as an error;
  the prompt still does not execute.
- **Empty `prompt` with a trigger** → direct 0-token delivery: the formatted
  trigger output (`goal_template`, or the raw output) is delivered to
  `deliver_to` without an agent turn at all.
- **`set_goal`** → the fired trigger dispatches into the target session as an
  active goal, so the session's agent works the condition it detected rather
  than just echoing output.

A trigger-gated job is the right shape for "watch X and act when it changes":
put the watching in `trigger_cmd` (deterministic, cheap, script-runner
pattern), the condition in `trigger_on`, and the acting in the prompt or goal.

## Using these templates

1. Copy a script into your workspace:
   ```
   cp health-check.sh ~/.opencrabs/scripts/
   chmod +x ~/.opencrabs/scripts/health-check.sh
   ```
2. Edit the `CONFIG` block at the top for your task.
3. Test it by hand: `~/.opencrabs/scripts/health-check.sh`
4. Create the cron job with the `cron_manage` tool, prompt =
   `"Run ~/.opencrabs/scripts/health-check.sh and reply with its output."`

## Decision guide

| Job shape | Author it as |
|-----------|--------------|
| One-shot simple reasoning ("summarize X in one line") | LLM prompt is fine |
| Multi-step logic, formatting rules, tool sequences | Script-runner pattern |
| Anything touching bash with real env needs | Script-runner pattern (script owns the env) |
| Needs memory of prior runs | Persist state to a file/DB; the session won't remember |

## What's in `scripts/`

| Script | What it does |
|--------|--------------|
| `health-check.sh` | Pings a list of URLs and checks disk usage, prints a status report. |
| `backup-rotate.sh` | Tars a source dir into a dated archive and prunes ones older than N days. |
| `digest.sh` | Aggregates lines from a log/source and prints a compact daily digest. |
