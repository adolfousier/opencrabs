# Decision Cache (`[decisions]`, #1648)

L1 of the decision pyramid: exact reuse of classification-shaped decisions.
Triage, routing, voice gates, draft scoring and self-audit calls are paid for
in full model price even when the input is an exact repeat. OpenCrabs already
has L0 rules/code-graph, L2 embeddings/FTS, L3 sub-agents and L4 prompt-cached
frontier models; the decision cache is the missing exact-reuse ring.

## How it works

The `decide_cached` tool takes three arguments:

| Arg | Meaning |
|-----|---------|
| `tier` | A configured ring name (`[decisions.tiers.<tier>]`) |
| `input` | The structured context the decision depends on (any JSON value; this is what is keyed) |
| `ask` | The question put to the model on a miss |

The cache key is `sha256(tier, policy_version, normalizer_version,
canonicalized_input)` joined by unit separators. Canonicalization sorts keys
and masks timestamps, UUIDs, IPs and paths, so a stored row can never be
served across a policy or normalizer change, and volatile fields cannot defeat
reuse.

Contract: the model answer must embed a JSON object (`decision`, optional `p`
confidence and `margin` runner-up distance). A reply that does not honor the
contract is a miss, never a guess, and is never cached.

## Modes

| Mode | Behavior |
|------|----------|
| `shadow` (default) | The model is ALWAYS asked, identical to a plain call, while would-hits are counted for the release-day evaluation |
| `live` | A fresh cached decision may be returned without a model call; the result then carries `"cached": true` |
| `off` | Kill switch: touches no cache, no counters and no shadow logs; a plain model call, the exact pre-feature path |

A tier with no config entry does not exist: calling it is a named error,
never an implicit default. No tier is ever promoted to `live` by code.

## Configuration

```toml
# One ring per decision family. policy_version is required.
[decisions.tiers.triage]
policy_version = "1"       # bump when the decision policy changes; invalidates old rows
mode = "shadow"            # shadow | live | off
ttl_hours = 336            # optional; rows older than this are pruned at startup
margin_floor = 0.2         # optional write gate; a reported margin below this is never cached
```

| Field | Type | Default | Notes |
|-------|------|---------|-------|
| `policy_version` | string | required | Missing or empty is a named load error. It is the invalidation lever: a policy change without a bump silently reuses stale decisions, the one failure the ring must never allow |
| `mode` | `shadow` \| `live` \| `off` | `shadow` | Promotion to `live` is an operator decision backed by measured shadow evidence |
| `ttl_hours` | int or absent | absent | `None` = no expiry on reads; set it so the sweeper has a bound |
| `margin_floor` | float | `0.0` | Borderline decisions stay live instead of being frozen at write time |
| `similarity` | bool | `false` | Unimplemented (L2 fuzzy reuse); `true` is rejected at load so an operator never believes a fuzzy ring is on |

## Accounting

- `/usage` prints a "Decision cache (#1648), per tier" block: calls,
  would-hit rate (shadow), live-hit rate, rows cached, and the estimated model
  calls avoided (live hits). It renders only where counters exist; on a fresh
  install the section is silent.
- Mission Control's report carries the same data as a "Decision cache" table
  (Tier / Calls / Would-hit / Live-hit), omitted when empty.
- A startup sweep prunes per-tier rows older than `ttl_hours`, expiring by
  `last_used_at` when set (a hit refreshes the row), falling back to
  `created_at`. Rows belonging to tiers removed from config are deleted; an
  emptied `[decisions.tiers]` set clears the table entirely (the kill switch
  also cleans up).
- Counters live in the `decision_stats` table with atomic per-call deltas, so
  concurrent sessions cannot lose counts to a read-modify-write race.

## Release-day evaluation (the kill rule)

The feature ships instrumented and unproven. Implemented now, evaluated at the
next release: unmeasured means removed, not extended.

Promotion bar, per tier: **>= 30% would-hit over >= 100 calls.**

1. Read the numbers: `/usage` decisions block, the Mission Control report, or
   `SELECT tier_id, calls, would_hit, live_hit FROM decision_stats;`.
2. Tier above the bar: candidate for `mode = "live"`. Promotion is Adolfo's
   manual call, one tier at a time, never automatic.
3. Tier below the bar or short of 100 calls: stays in shadow (data keeps
   accruing) or is removed at the operator's judgment.
4. Removal: set `mode = "off"` (or delete the section) so no code path reads
   or writes, then `DROP TABLE IF EXISTS decision_cache; DROP TABLE IF EXISTS
   decision_stats;`. Nothing else references either table
   (migrations `20260921000001_add_decision_cache.sql` and
   `20260921000002_add_decision_stats.sql`).

## Storage

`decision_cache` columns: `key` (PK), `tier_id`, `result_json`, `p`, `margin`,
`policy_version`, `normalizer_version`, `hits`, `created_at`, `last_used_at`.
The cache is a reuse ring for cheap repeatable calls, not a system of record:
dropping it at any time is safe.
