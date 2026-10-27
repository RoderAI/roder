# Prompt admission and recovery

The TUI retains each submitted prompt, including structured image inputs and
its original thread, until the backend returns an admission receipt.

## Queue holds

Queued follow-ups wait while:

- a turn is active;
- a start or steer request is awaiting a response;
- a successful start receipt is awaiting its lifecycle event;
- a provider/model/settings selection, approval, or user-input dialog is open;
- a plan-exit decision is pending; or
- source-copy mode is open.

Closing a selection or copy mode releases its hold. Dispatch is centralized in
the update loop, so multiple UI events cannot each start the same queued input.
Each thread has its own visible queue. Resuming another thread does not discard
the first thread's queued or recoverable inputs.

## Definite rejection versus uncertain acceptance

A steer that explicitly reports no active target is retained for a new turn,
once. A newer active turn is never cleared by an older rejected steer. Recovery
does not append a second optimistic transcript entry for the same input.

Other rejections retain their exact input for `/retry`, in original submission
order even when responses arrive out of order. Retrying one input does not clear
later queued messages.
Use `/retry discard` to explicitly drop the oldest retained input for the current
thread, for example after a permanent rejection. This releases its recovery hold
without deleting later queued input or another thread's recovery state.

A lost, malformed, cancelled, or timed-out response does **not** prove rejection.
The TUI never automatically resends that input. The diagnostic asks the user to
inspect thread history before explicitly retrying. A request deadline releases
the pending-request state into recoverable input rather than dropping it.
Client request IDs are correlation identifiers, not a claim of server-side
idempotency.

## Lifecycle reconciliation

A successful start receipt holds subsequent input until the corresponding
start/completion is observed. If an event was lost, the TUI performs bounded,
asynchronous metadata reads and checks the persisted lifecycle record. Terminal
evidence releases the hold; an authoritative active-turn record restores the
working state. A late receipt cannot resurrect a turn already observed terminal.

The queue and recovery state are TUI-session state, not a durable offline outbox.
Closing the application does not promise recovery of input never accepted by
the backend.
