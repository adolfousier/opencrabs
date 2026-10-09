# Governor policy matrix — the frozen contract (UPSTREAM re-cite)

**Issue:** [opencrabs/opencrabs#1927](https://github.com/opencrabs/opencrabs/issues/1927) — the wider engine that unblocks fork [#556](https://github.com/leshchenko1979/opencrabs/issues/556) / [#580](https://github.com/leshchenko1979/opencrabs/issues/580) / [#635](https://github.com/leshchenko1979/opencrabs/issues/635).
**Base:** upstream `main` @ `fb28a79b18e4f80e0458aabf91754ca81545509d` (worktree `up-635-pace-engine`, branch `leshchenko1979/refactor/pace-engine-635`).
**Purpose:** every later step of the engine consolidation is diffed against this table. A cell that changes is a
behavioural change and must be named as one. A cell that is missing is a bug in this document.
**Provenance:** re-cited from the fork's frozen matrix (fork commit `eab049ae1`, 120 fork-line citations) against
the *upstream* line numbers read this turn. The fork matrix is the design record; THIS file is the diff target.
**File under review:** `governor.rs` = `src/channels/telegram/governor.rs` (**1339 lines**) unless a cell says otherwise.
`rate_limit.rs` = `src/channels/telegram/rate_limit.rs` (**200 lines**).

---

## 0. Census — the duplication this refactor must reduce

| Measure | Baseline (upstream `main` @ `fb28a79b1`, read this turn) |
|---|---|
| `governor.rs` lines | 1339 |
| `peers()` occurrences | 17 (1 def @ governor.rs:407, 10 production lock sites, 3 fold `get_mut` sites, 3 in `#[cfg(test)]`) |
| production `peers().lock()` sites | 10 — consumers/loops 452/777/805/823/871, gates 519/707/1017/1114, test-only 1216 |
| `or_default()` | 7 |
| `fold_*` helpers | 3 (`fold_throttle_ms` governor.rs:578, `fold_rich_ms` governor.rs:1159, `fold_send_ms` governor.rs:1178) |
| gate sleep+advance pairs | 3 (governor.rs:563/569, governor.rs:1062/1068, governor.rs:1145/1151) + `acquire_global_permit` governor.rs:160/158 + drainer governor.rs:795/800 |
| verdict shapes | 4 (`bool` G1, `bool` G2, `()` G3, `()` G4) |
| `Counters` fields | 16 (governor.rs:332-347) |
| `RateLimiterConfig` knobs | 12 (config/types.rs) mirrored by hand in `Limits` (governor.rs:173-198) |

---

## 1. The four gates — one row each

| # | Gate (fn) | Surface | Buckets | Floor policy | On-dry policy | Max hold | Fail-open | Return | Counters |
|---|---|---|---|---|---|---|---|---|---|
| G1 | `admit_chat_action` @ governor.rs:499 | `sendChatAction` (typing) | `peer.typing`, burst `typing_burst`, refill `1/typing_interval` — governor.rs:527-531 | **EXEMPT** — no spacing call at all | drop when `waited + wait > typing_max_hold` — governor.rs:538-541 | `typing_max_hold` — governor.rs:538 | n/a — the drop IS terminal | `bool` — governor.rs:499 | `admitted_typing` governor.rs:534; `dropped_typing` governor.rs:539; `throttled_typing_ms` via `fold_throttle_ms` governor.rs:578 |
| G2 | `edit_admission_media_kb` @ governor.rs:677 (`edit_admission` @ governor.rs:650 delegates) | `editMessageText` / rich edits | `peer.edits`, burst `edit_burst`, refill `edit_rate_per_sec` — governor.rs:715 | **droppable only** — `class` ladder; Interactive never drops | `Final` → queue latest-wins governor.rs:722-742; chrome → `note_drop` governor.rs:744-745; `Interactive` floor-dry → pass through governor.rs:719-721 | none — no wait loop; G2 returns immediately | n/a — `Final` queues, chrome drops | `bool` — governor.rs:677 | `admitted_edits` governor.rs:717; `queued_finals`/`superseded_finals` governor.rs:739-741; `dropped_*` via `note_drop` governor.rs:353-363 |
| G3 | `pace_send` @ governor.rs:996 | `sendMessage` | **two AND-ed**: `peer.sends_sec` (burst `send_burst`, refill `1/send_interval`) + `peer.sends_min` (capacity `send_minute_ceiling`) — governor.rs:1022-1033 | **WAITED** — `need = sec.next_token_in().max(min.next_token_in())` — governor.rs:1033 | `need == 0` → take both, admit governor.rs:1035-1038; `waited + need > SEND_MAX_HOLD` → **admit anyway** governor.rs:1039-1041; else wait | `SEND_MAX_HOLD = 30s` — governor.rs:89 | **YES**, past 30 s — admit + `FailOpen(need)` — governor.rs:1039-1041 | `()` — governor.rs:996 | `admitted_sends` governor.rs:1037 **and** governor.rs:1040; `throttled_send_ms` via `fold_send_ms` governor.rs:1178 |
| G4 | `pace_rich` @ governor.rs:1092 | `sendRichMessage` + rich edits | `peer.rich`, burst `rich_burst`, refill `rich_rate_per_sec` — governor.rs:1127 | **never drops** — waits | `need == 0` → `bucket.take` → admit governor.rs:1130-1134; else wait governor.rs:1136 | none — waits until a token refills | **NEVER** | `()` — governor.rs:1092 | `admitted_rich` governor.rs:1132; `throttled_rich_ms` via `fold_rich_ms` governor.rs:1159 |

### Shared pre-gate stages (the duplicated skeleton)

All four gates run the same stages in the same order. The only per-gate differences are the
`forum_seen` set-condition, whether the global permit is consulted, and the global-cooldown fast path:

| Stage | G1 (governor.rs:499) | G2 (governor.rs:677) | G3 (governor.rs:996) | G4 (governor.rs:1092) |
|---|---|---|---|---|
| global-cooldown fast path | `is_global_cooldown_active()` → false — governor.rs:501-503 | active && class ∉ {Final, Interactive} → false — governor.rs:689-693 | — (not present) | — (not present) |
| `acquire_global_permit()` | **absent** | **absent** | `acquire_global_permit().await` governor.rs:1007 | `acquire_global_permit().await` governor.rs:1103 |
| DM guard `chat_id >= 0` | → true governor.rs:508-510 | → true governor.rs:697-699 | → return governor.rs:999-1001 | → return governor.rs:1095-1097 |
| `Limits::from_config()` + `!enabled` | → true governor.rs:512-514 | → true governor.rs:701-703 | → return governor.rs:1010-1012 | → return governor.rs:1106-1108 |
| `ensure_summary_task()` | governor.rs:515 | governor.rs:704 | **absent** | governor.rs:1110 |
| `peers().lock()` + `entry(chat).or_default()` | governor.rs:519-520 | governor.rs:707-708 | governor.rs:1017-1018 | governor.rs:1114-1115 |
| `forum_seen` set / bail | set iff `thread_id.is_some()` governor.rs:521-522; bail → true governor.rs:524-526 | **no set**; bail → true governor.rs:712-714 | **no set**; bail → return governor.rs:1019-1021 | set iff `thread_id.is_some()` governor.rs:1121-1122; bail → return governor.rs:1124-1126 |
| `gate_now()` | governor.rs:532 | governor.rs:705 | governor.rs:1032 | governor.rs:1128 |
| sleep + test-advance + fold | governor.rs:563/569, `fold_throttle_ms` governor.rs:578 | — (no loop) | governor.rs:1062/1068, `fold_send_ms` governor.rs:1178 | governor.rs:1145/1151, `fold_rich_ms` governor.rs:1159 |

**Note.** Upstream's G3/G4 consult `acquire_global_permit() -> bool` (governor.rs:123); the fork's engine
consults a `GlobalPermit` verdict. Upstream's `edit_admission` takes `(bot, chat_id, msg_id, class, html, rich)`
(governor.rs:650-658); the fork takes an `EditPayload`. These signature divergences are the sharpest edge of
the port and are why the wrappers exist — the public call sites do not move.

---

## 2. The three pause concepts (step 5's target)

| # | Concept | Where declared | Where enforced | Scope |
|---|---|---|---|---|
| P1 | global 429 deadline | `rate_limit.rs` — `MAX_INLINE_RATE_LIMIT_WAIT = 30s` — rate_limit.rs:65 | `is_global_cooldown_active` / `wait_global_cooldown`, read at governor.rs:501, governor.rs:689, governor.rs:125 | process-wide |
| P2 | per-chat 429 pause | **ABSENT upstream** — no `note_429_pause`, no `pause_armed_429`, no `MAX_429_PAUSE` (0 hits in `src/`) | — | per-chat, per-arm (fork-only) |
| P3 | bucket pause field | **ABSENT upstream** — `Bucket` (governor.rs:226-232) has no `pause_until`; `take` (governor.rs:251-260) is a plain consume | — | per-bucket (fork-only) |

`Bucket::take` upstream returns `Result<(), Duration>` with no pause branch — governor.rs:251-260.
The fork's `Bucket::refill`/`take` pause-freeze and `take_any` bypass do not exist upstream (`take_any` survives
only as a **comment** at governor.rs:138-139 explaining that upstream's floor-free `take` is what the fork's
`take_any` provided). P2/P3 are therefore the **new behaviour** this port introduces, not a consolidation of
existing upstream concepts — the step-5 "collapse three pause concepts" reads differently here: it ADDS the
unified `Cooldown` and routes P1 (and the new P2/P3) through it.

---

## 3. Live defect present upstream (feeds step 6)

`Counters` declares **16** fields (governor.rs:332-347). `format_summary` emits all 16 (governor.rs:414-445).
`all_zero()` (governor.rs:365-383) tests only **14** — it omits:

- `admitted_rich`
- `throttled_rich_ms`

Consequence: a peer whose only activity is rich traffic is judged "all zero" and its periodic summary line is
**suppressed** — governor.rs:415-416. Read this turn by structural parse, not by eye.

---

## 4. The config mirror (step 7's target)

`RateLimiterConfig` declares **12** knobs (`config/types.rs`). `Limits` declares **12** fields
(governor.rs:173-198) and `Limits::from_config` (governor.rs:200-222) reads exactly those 12 `rl.*` names —
a hand-maintained 1:1 mirror with no compiler link.

---

## 5. What step 4 must preserve, bit for bit

- G1: hold-and-release loop; drop past `typing_max_hold`; `fold_throttle_ms` on BOTH admit and drop.
- G2: no loop; Interactive passes through on a dry bucket; Final queues latest-wins and spawns the drainer;
  chrome drops via `note_drop` and the drop ladder rank is `drop_rank()` (governor.rs:624-635).
- G3: two AND-ed buckets; **fails open** past `SEND_MAX_HOLD` and still counts `admitted_sends`.
- G4: one bucket; **never** fails open; counts `admitted_rich`.
- Every gate's `forum_seen` set-condition is per-surface and must not be unified into one shape.
