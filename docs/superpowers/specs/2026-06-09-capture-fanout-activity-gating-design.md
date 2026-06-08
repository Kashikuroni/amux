# Design: capture-pane fan-out reduction via activity-gating

- **Date:** 2026-06-09
- **Status:** approved (brainstorm), pre-implementation
- **Topic:** the "main lever" residual from the 2026-06-08 performance pass —
  `capture-pane` fan-out in `refresh()`.

## Problem

`App::refresh()` (currently `src/app.rs:2484`) forks one short-lived `tmux`
client per managed session per tick, plus a scrollback capture for the selected
session, plus the `list-sessions` fork:

```
per tick = 1 (list-sessions) + N (capture-pane, one per session) + 1 (capture-scrollback, selected)
         = N + 2 forks
```

The default refresh interval is 1500 ms (`config.refresh_interval_ms`), so with
N sessions this is `(N+2)` process spawns every 1.5 s, **linear in session
count**. Each fork is a tmux client spawn + a socket round-trip. The
2026-06-08 pass (F1–F5) gated *git* re-reads but left the pane-capture fan-out
untouched; it is the dominant remaining fork cost and the last large idle-cost
lever.

### Goals

Both, per the brainstorm:

1. **Lower the idle floor now** — fewer forks/wakeups at the current ~5-session
   scale.
2. **Remove linear growth** — cost should track *active* sessions, not the
   total session count, so 20–50 sessions don't multiply the fork rate.

### Non-goals (explicitly out of scope)

- Strictly zero forks per tick (that is the future control-mode upgrade).
- Instant (sub-poll-interval) status latency (also control mode).
- Changing the refresh interval.
- Batching multiple changed captures into one fork (noted as a possible later
  refinement; YAGNI for now).

## Chosen approach: activity-gating

Mirror the existing F3 git-gating philosophy — *gate an expensive operation
behind a cheap change signal* — applied to pane capture.

tmux exposes `#{session_activity}` (and `#{window_activity}`) as an
epoch-seconds timestamp of the session's last activity, readable via
`list-sessions -F`. Verified on tmux 3.6b:

```
$ tmux -L probe new-session -d -s probe 'sleep 5'
$ tmux -L probe list-sessions -F '#{session_name} #{session_activity}'
probe 1780955306
```

Because it rides the `list-sessions` fork we already make, obtaining the change
signal costs **zero extra forks**. We then `capture-pane` only the sessions
whose activity advanced (plus a conservative set, below), and reuse cached
state for the rest.

### Why this fits `compute_status`

`compute_status` is already a pure "did the content change?" function:

```rust
pub fn compute_status(prev: Option<u64>, current: u64) -> Status {
    match prev { Some(p) if p != current => Status::Running, _ => Status::Idle }
}
```

`Running ⟺ content changed`, `Idle ⟺ unchanged`, and `Waiting` is overlaid
separately when `parse_prompt` finds a numbered menu. "Activity advanced"
is exactly equivalent to "content changed", so the activity signal maps onto
the existing status machine without changing its semantics.

## Capture rule

Capture `capture-pane` for a session if **any** of:

1. **New session** — no entry in `last_activity` (first observation).
2. **Activity advanced** — `activity > last_seen[name]` (output happened since
   last tick; content definitely changed).
3. **Previous status was `Running`** — conservative top-up.

Otherwise (Idle/Waiting with unchanged activity) → **skip the capture**, reuse
cached state.

### Why rule 3 is required

`session_activity` has **1-second** granularity; the tick is 1.5 s. Narrow
sub-second race: output that lands in the *same wall-clock second* as the
previously recorded activity time, but *after* our read, leaves the timestamp
unchanged — so rule 2 alone would miss that change until the next activity.

Rule 3 closes the race for every genuinely-active session: an agent producing
output mid-burst is `Running`, so it is always re-read. Idle sessions — the
whole point of the optimization — stay gated. Cost: when an agent goes quiet it
takes **one extra confirming capture** before it gets gated, then it is skipped.
Principle: an extra capture is cheap; a missed change (stale status) is not.

Worked transition (agent going quiet):

| tick | activity | prev status | action | result |
|------|----------|-------------|--------|--------|
| 1 | 100 (new output) | — | capture (rule 2) | hash changes → Running |
| 2 | 100 (final output, same sec) | Running | capture (rule 3) | hash changes → Running |
| 3 | 100 (quiet) | Running | capture (rule 3) | hash unchanged → Idle |
| 4 | 100 (quiet) | Idle | **skip** | Idle (cached) |

## Status mapping for skipped sessions (pure logic, no fork)

When a session is skipped, its screen is identical to the previous tick, so:

- status = `Waiting` if the cached prompt is non-empty, else `Idle`.

This reproduces bit-for-bit what a re-capture would yield: `compute_status` on
an unchanged hash returns `Idle`, and a present prompt overlays `Waiting`.

## State and `refresh()` integration

### New `App` field

`last_activity: HashMap<String, i64>` — session name → last seen
`session_activity`. Direct analogue of the existing `git_last_enqueue`,
including cleanup: at the end of `refresh()`, `retain` by live session names,
alongside the existing `snapshots` / `git_last_enqueue` pruning.

### New `Session` field

`activity: i64` (epoch seconds), parsed from the extended `LIST_FORMAT` in
`src/tmux.rs`.

### Capture-loop rewrite (`src/app.rs`, the `for s in &mut sessions` block)

Today the loop unconditionally captures every session and rebuilds
`new_snaps` / `new_prompts` from scratch. New shape:

- compute `should_capture(...)` per session;
- **capture** → unchanged from today: hash, `compute_status`, `parse_prompt`,
  insert into `new_snaps` / `new_prompts`, set `last_activity[name] = activity`;
- **skip** → carry forward from the previous tick: `new_snaps[name] = old
  snapshot`, `new_prompts[name] = old prompt` (if any), status via the pure
  idle-mapping above. Do **not** touch `last_activity`.

**Completeness invariant:** after a tick, `new_snaps` (and `new_prompts`) must
hold an entry for **every** live session — captured *and* skipped — otherwise
`self.snapshots = new_snaps` drops the skipped sessions' history and the next
tick falsely treats them as changed. Enforced by an explicit test.

### Preview

Capture the selected session's scrollback only when that session is in the
captured set this tick (its activity advanced). Selection moves onto a quiet
session are already covered by the Part C debounce (`update_preview()` in
`src/main.rs`), so the preview does not go stale on navigation.

## Pure helpers (unit-tested in isolation, F3 style)

- `should_capture(activity: i64, last_seen: Option<i64>, prev_status: Status) -> bool`
  — the entire capture rule.
- `status_when_idle(cached_prompt_present: bool) -> Status` — the skipped-session
  mapping (`Waiting` / `Idle`).

## Testing

Unit (pure), matching the F3 / Part B style:

- `should_capture`: new session → true; `activity > last_seen` → true;
  `activity == last_seen` + prev `Idle` → false; `activity == last_seen` +
  prev `Running` → true (rule 3); `activity < last_seen` (clock rollback,
  theoretical) → false unless rule 3 tops it up.
- `status_when_idle`: prompt present → `Waiting`; absent → `Idle`.
- **Completeness invariant** (key regression test): after a tick where some
  sessions are skipped, `snapshots` and `prompts` contain an entry for every
  live session, and a skipped session does not flip to `Running` next tick.
- Running → Idle transition: an active session that goes quiet gets one
  confirming capture, then is gated, status `Idle`.
- `parse_sessions` parses the new `activity` field from `list-sessions -F`
  (extend the existing parser test).

## Expected outcome

- All sessions quiet → **1 fork/tick** (just `list-sessions`), down from `N+2`.
- Otherwise → `1 + (changed sessions) + (1 if the preview changed)`.
- Fork cost tracks *active* sessions, independent of total count.
- Model stays **stateless and self-healing**: no long-lived process, no
  protocol parser, no reconnect. The change is localized to `refresh()` plus
  two pure helpers and the `Session` / `LIST_FORMAT` field.

## Future upgrade: tmux control mode

If poll-interval status latency (1.5 s) becomes noticeable, or a strict
zero-fork steady state is wanted, the endgame is **tmux control mode**
(`tmux -C`) — the model iTerm2's tmux integration uses:

- One long-lived control client holds the connection open. Commands are writes
  to its stdin; replies come back framed in `%begin … %end` blocks — **no
  process spawn per command**.
- The server pushes notifications unprompted: `%output %<pane> <data>` (live
  pane output), `%window-add`, `%window-close`, `%session-changed`,
  `%layout-change`, etc. Status becomes truly event-driven and instant; the
  periodic refresh timer could be dropped entirely.

Costs / new failure modes that make it a separate, larger project:

- A `%begin/%end` protocol parser + command-reply matching by sequence number +
  `%error` handling + notification demux (a real state machine, though
  unit-testable on canned protocol text).
- Long-lived subprocess lifecycle: spawn, death detection, **reconnect** after
  a server restart / crash / `amux doctor --clean`. Trades today's independent,
  self-healing ticks for a stateful connection that can desync.
- `%output` is a firehose: a chatty agent streams continuously. Naively
  subscribing to all panes' output may process *more* data than today's bounded
  snapshots. Mitigation: use control mode as a cheap command channel + structural
  notifications only, suppressing pane output — a "middle" design decision to
  make then.
- A persistent control client is a second client on the server: it has a size
  and can resize panes (`refresh-client -C`), intersecting amux's existing
  per-preview resize management and interactive attach.

Trigger to revisit: poll latency matters, or fork count must be strictly zero.

## Possible later refinements (not now)

- **Batch changed captures into one fork** via a `;`-chained `capture-pane`
  sequence with delimiters. Marginal at typical load (1–2 active sessions/tick)
  and adds concatenated-output parsing; deferred.
