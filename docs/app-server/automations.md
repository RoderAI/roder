# Roder App-Server Automations

Automations are app-server-managed scheduled Roder runs. They are not a global
side effect of launching the CLI or TUI. A client can always read automation
definitions and run history when the read API is enabled, but only an app-server
instance with scheduler enablement owns ticking, missed-run recovery, leases,
and background run submission.

## Enablement

Scheduling is disabled by default. Enable it for the app-server instance that
should own scheduled execution:

```sh
roder app-server --enable-automations --automation-server-id desktop-main --automation-server-role desktop
```

Desktop clients may opt into scheduler ownership. Ordinary TUI-local
app-server instances should remain scheduler-disabled unless the user explicitly
requests local scheduled execution. A disabled scheduler can still answer
`automations/status`, `automations/list`, and `automations/runs` when
`readApiEnabled` is true.

Config and environment inputs:

| Config or env | Meaning |
| --- | --- |
| `app_server.automations.enabled`, `RODER_AUTOMATIONS_ENABLED` | Whether this instance starts the scheduler. |
| `app_server.automations.server_id`, `RODER_AUTOMATIONS_SERVER_ID` | Stable owner id recorded on leases and runs. |
| `app_server.automations.server_role`, `RODER_AUTOMATIONS_SERVER_ROLE` | Client role label such as `desktop`, `cli`, or `tui`. |
| `app_server.automations.store`, `RODER_AUTOMATIONS_STORE` | SQLite store path for definitions, occurrences, leases, and runs. |

## Methods

### `automations/status`

Purpose: Read scheduler ownership and run counters for this app-server
instance.

Request:

```json
{}
```

Response:

```json
{
  "schedulerEnabled": false,
  "readApiEnabled": true,
  "serverId": "desktop-main",
  "serverRole": "desktop",
  "storePath": "/Users/example/.roder/automations.sqlite3",
  "activeRuns": 0,
  "dueCount": 0,
  "leasedCount": 0
}
```

Clients distinguish states as follows:

- Scheduler disabled: `schedulerEnabled: false`.
- Idle: scheduler enabled, `activeRuns`, `dueCount`, and `leasedCount` are 0.
- Due work: `dueCount` is greater than 0.
- Running work: `activeRuns` is greater than 0.
- Leased work: `leasedCount` is greater than 0.
- Failed or missed work: inspect `automations/runs` for `failed` or `skipped`
  runs and their `error` or `skipReason`.

### `automations/create`

Purpose: Register an automation definition against a project.

Request:

```json
{
  "name": "Hourly project check",
  "project": {
    "cwd": "/Users/example/project",
    "displayName": "project"
  },
  "schedule": {
    "interval": {
      "seconds": 3600
    }
  },
  "prompt": "Report repository status using only local git and test commands.",
  "enabled": true,
  "modelProvider": "codex",
  "model": "gpt-5.5",
  "catchUp": {
    "runLatestOnly": null
  },
  "concurrency": "forbid"
}
```

Response:

```json
{
  "automation": {
    "id": "automation-123",
    "name": "Hourly project check",
    "project": {
      "cwd": "/Users/example/project",
      "displayName": "project"
    },
    "schedule": {
      "interval": {
        "seconds": 3600
      }
    },
    "prompt": "Report repository status using only local git and test commands.",
    "enabled": true,
    "modelProvider": "codex",
    "model": "gpt-5.5",
    "catchUp": {
      "runLatestOnly": null
    },
    "concurrency": "forbid",
    "createdBy": {
      "id": "app-server",
      "kind": "app_server"
    },
    "createdAt": "2026-05-21T10:00:00Z",
    "updatedAt": "2026-05-21T10:00:00Z"
  }
}
```

### `automations/update`

Purpose: Patch fields on an existing automation.

Request:

```json
{
  "automationId": "automation-123",
  "patch": {
    "enabled": false,
    "prompt": "Report repository status and failing checks."
  }
}
```

Response:

```json
{
  "automation": {
    "id": "automation-123",
    "name": "Hourly project check",
    "project": {
      "cwd": "/Users/example/project"
    },
    "schedule": {
      "interval": {
        "seconds": 3600
      }
    },
    "prompt": "Report repository status and failing checks.",
    "enabled": false,
    "catchUp": {
      "runLatestOnly": null
    },
    "concurrency": "forbid",
    "createdBy": {
      "id": "app-server",
      "kind": "app_server"
    },
    "createdAt": "2026-05-21T10:00:00Z",
    "updatedAt": "2026-05-21T10:05:00Z"
  }
}
```

### `automations/list`

Purpose: List registered automation definitions.

Request:

```json
{}
```

Response:

```json
{
  "automations": [
    {
      "id": "automation-123",
      "name": "Hourly project check",
      "project": {
        "cwd": "/Users/example/project"
      },
      "schedule": {
        "interval": {
          "seconds": 3600
        }
      },
      "prompt": "Report repository status using only local git and test commands.",
      "enabled": true,
      "catchUp": {
        "runLatestOnly": null
      },
      "concurrency": "forbid",
      "createdBy": {
        "id": "app-server",
        "kind": "app_server"
      },
      "createdAt": "2026-05-21T10:00:00Z",
      "updatedAt": "2026-05-21T10:00:00Z"
    }
  ]
}
```

### `automations/runNow`

Purpose: Queue an immediate run through the same lease, task, thread, and audit
path as scheduled occurrences.

Request:

```json
{
  "automationId": "automation-123",
  "promptOverride": "Run the hourly check now with mock-safe commands only."
}
```

Response:

```json
{
  "run": {
    "runId": "run-123",
    "automationId": "automation-123",
    "occurrenceKey": "automation-123:manual:2026-05-21T10:10:00Z",
    "state": "queued",
    "scheduledFor": "2026-05-21T10:10:00Z",
    "queuedAt": "2026-05-21T10:10:00Z",
    "serverId": "desktop-main",
    "serverRole": "desktop"
  }
}
```

### `automations/runs`

Purpose: Read run history, optionally filtered by terminal or active state.

Request:

```json
{
  "automationId": "automation-123",
  "state": "failed",
  "limit": 20
}
```

Response:

```json
{
  "runs": [
    {
      "runId": "run-123",
      "automationId": "automation-123",
      "occurrenceKey": "automation-123:manual:2026-05-21T10:10:00Z",
      "state": "failed",
      "scheduledFor": "2026-05-21T10:10:00Z",
      "queuedAt": "2026-05-21T10:10:00Z",
      "startedAt": "2026-05-21T10:10:02Z",
      "finishedAt": "2026-05-21T10:10:05Z",
      "threadId": "thread-123",
      "turnId": "turn-123",
      "taskId": "task-123",
      "serverId": "desktop-main",
      "serverRole": "desktop",
      "error": "automation run blocked waiting for interactive input"
    }
  ]
}
```

### `automations/cancelRun`

Purpose: Mark a queued or running automation run as cancelled.

Request:

```json
{
  "runId": "run-123",
  "reason": "User cancelled from desktop"
}
```

Response:

```json
{
  "runId": "run-123",
  "cancelled": true
}
```

### `automations/delete`

Purpose: Soft-delete an automation by disabling it while preserving run history.

Request:

```json
{
  "automationId": "automation-123"
}
```

Response:

```json
{
  "automationId": "automation-123",
  "deleted": true
}
```

## Scheduler Behavior

Schedules support interval, cron with timezone, and one-shot definitions. The
scheduler expands missed occurrences from the last checked timestamp. Catch-up
policy controls whether missed occurrences become multiple due runs, a single
latest run, or skipped records with `state: "skipped"` and `skipReason`.

Concurrency policy controls what happens when a previous occurrence is still
active. Leases record `serverId`, `serverRole`, `leasedAt`, and `expiresAt`.
Expired leases can be recovered by a later scheduler tick, so a client should
use run state and lease counters rather than assuming a process crash loses
scheduled work.

Automation workers create normal Roder sessions and turns. If a run asks for
approval or user input, the worker interrupts the turn and records a failed run
with `error: "automation run blocked waiting for interactive input"`.

## Notifications

Automation lifecycle notifications use JSON-RPC notification envelopes:

```json
{
  "jsonrpc": "2.0",
  "method": "automations/runStarted",
  "params": {
    "run": {
      "runId": "run-123",
      "automationId": "automation-123",
      "occurrenceKey": "automation-123:manual:2026-05-21T10:10:00Z",
      "state": "running",
      "scheduledFor": "2026-05-21T10:10:00Z",
      "threadId": "thread-123",
      "turnId": "turn-123",
      "taskId": "task-123"
    }
  }
}
```

Notification methods:

| Method | Meaning |
| --- | --- |
| `automations/runStarted` | A run entered `running`. |
| `automations/runCompleted` | A run completed successfully. |
| `automations/runFailed` | A run failed with an error. |
| `automations/runSkipped` | A missed or suppressed occurrence was skipped. |
| `automations/needsInput` | A failed run blocked on approval or user input. |

## Mock-Safe Examples

Hourly project check:

```json
{
  "name": "Hourly project check",
  "project": {
    "cwd": "/Users/example/project"
  },
  "schedule": {
    "interval": {
      "seconds": 3600
    }
  },
  "prompt": "Run git status --short and report whether local tests need attention. Do not modify files.",
  "enabled": true,
  "catchUp": {
    "runLatestOnly": null
  },
  "concurrency": "forbid"
}
```

Daily summary:

```json
{
  "name": "Daily summary",
  "project": {
    "cwd": "/Users/example/project"
  },
  "schedule": {
    "cron": {
      "expression": "0 9 * * *",
      "timezone": "Europe/London"
    }
  },
  "prompt": "Summarize yesterday's committed changes from git log --since yesterday. Do not run network commands.",
  "enabled": true,
  "catchUp": {
    "skipExpired": {
      "graceSeconds": 7200
    }
  },
  "concurrency": "forbid"
}
```

Weekly roadmap refresh:

```json
{
  "name": "Weekly roadmap refresh",
  "project": {
    "cwd": "/Users/example/project"
  },
  "schedule": {
    "cron": {
      "expression": "0 10 * * MON",
      "timezone": "Europe/London"
    }
  },
  "prompt": "Read roadmap/STATUS.md and list unchecked items that still need implementation or validation. Do not edit files.",
  "enabled": true,
  "catchUp": {
    "runLatestOnly": null
  },
  "concurrency": "forbid"
}
```
