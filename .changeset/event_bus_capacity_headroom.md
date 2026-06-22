---
roder-core: patch
---

# Increase the runtime event-bus capacity so heavy turns stop dropping rows

The runtime broadcasts every event (including a per-delta `InferenceEventReceived`
firehose) through a single broadcast ring buffer. The TUI drains it on its render
loop, which during an active turn only wakes at the ~6 FPS status-animation
cadence — backend events do not wake the input poll, so up to ~166ms of events
buffer between drains. A reasoning-high provider that runs its whole tool loop in
one turn (e.g. `claude-code`) bursts far more than 1024 events into that window,
overflowing the ring and silently dropping tool/thinking rows from the live view.

The bus capacity is raised from 1024 to 16384 (`EVENT_BUS_CAPACITY`), giving ~16x
headroom across streaming bursts and brief render stalls. The existing `Lagged`
handling and stuck-turn watchdog still cover any pathological overflow, so the UI
can never hang even if the buffer is exhausted.
