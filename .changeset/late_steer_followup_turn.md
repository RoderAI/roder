---
roder-core: patch
---

# Deliver input sent as a turn finishes instead of dropping it

Messages steered into a turn after its last model call, or queued prompts a
host sends right after `TurnCompleted` while the turn is still tearing down,
were accepted and then silently discarded. The runtime now carries such
unconsumed input into a follow-up turn (or into a newer turn already running
on the thread), ahead of any automatic goal continuation.
