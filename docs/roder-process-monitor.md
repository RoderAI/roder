# Roder Process Monitor

Roder tracks processes it owns: `command/exec`, process-backed background tasks, and remote runner commands. It does not list arbitrary host OS processes.

## Surfaces

- TUI slash command: `/ps`, `/ps all`, `/ps <process-id>`, `/ps stop <process-id>`, `/ps stop-all --confirm`
- Ctrl+P: `Processes` opens the same inventory, with detail and stop actions for individual processes
- CLI: `roder ps`, `roder ps --all`, `roder ps show <process-id>`, `roder ps stop <process-id>`, `roder ps stop-all --confirm`

`/tasks` remains task-centric history. `/ps` is for process ownership, output tails, and stop controls. Process-backed tasks include their task id so both views can be correlated.

## Local Smoke Flow

Use an app-server process task to create a long-running process, list it, inspect output, and stop it:

```sh
roder tasks submit process '{"command":"sh","args":["-c","printf ready\\n; sleep 30"],"cwd":"."}'
roder ps --all
roder ps show <process-id>
roder ps stop <process-id>
roder ps --all
```

For the interactive TUI:

```sh
/ps
/ps all
/ps <process-id>
/ps stop <process-id>
/ps stop-all --confirm
```

## Remote Runners

Remote process rows include `runnerDestinationId` and `runnerSessionId`. Stopping a remote process calls the runner provider cancellation API. Roder never tries to kill a local PID for a remote runner process.

Live remote-runner verification is opt-in:

```sh
RODER_LIVE_REMOTE_RUNNER=1 roder tasks submit process '{"command":"sh","args":["-c","sleep 30"]}'
```
