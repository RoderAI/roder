# Roder Dynamic Workflows

Dynamic workflows are approval-gated orchestration scripts for work that should
coordinate multiple child agents without filling the lead conversation context.
They are separate from workflow imports: imports use the singular
`workflow/*` app-server namespace, while dynamic workflow runs use
`workflows/*`.

## Triggering

- Prompt trigger: include the word `workflow` in a substantive prompt. The TUI
  shows a trigger hint before submit and lets the user ignore it once with Esc.
- Saved command: run a saved `.workflow.js` command from `.agents/workflows/`
  or `~/.roder/workflows/`.
- Built-in command: `/deep-research <question>` uses the dynamic workflow
  runner with fixture search in tests and live web search only when explicitly
  enabled.
- App-server or SDK: call `workflows/plan` and then `workflows/approve`.
- Effort mode: `/effort ultracode` enables automatic workflow planning for
  substantive tasks when `dynamic_workflows.auto_with_ultracode` is true.

Tiny prompts, slash commands, approval replies, and pure chat questions do not
auto-trigger workflow planning.

## Approval And Safety

The first run of a generated or saved script shows the planned phases, child
agent estimate, scope, limits, and a script preview before execution. Approval
decisions are `runOnce`, `alwaysForScriptAndWorkspace`, or `deny`.

Workflow scripts run inside a constrained JavaScript runtime. They can
coordinate `ctx.agents`, `ctx.phase`, `ctx.checkpoint`, and `ctx.report`, but
cannot directly read files, run shell, access the network, inspect secrets, load
dynamic modules, or call MCP. Child agents do the actual work through normal
Roder policy, lane, tool, and permission checks.

## Runtime Controls

Use `/workflows` in the TUI or the `workflows/*` app-server methods to list,
inspect, pause, resume, stop, restart child agents, and save scripts. Progress
summaries include run status, phase counts, agent counts, failed children,
concurrency peak, elapsed time, token usage when available, and report preview.

Saved scripts are `.workflow.js` files:

- Workspace: `.agents/workflows/`
- User: `~/.roder/workflows/`

Saving a script does not bypass future approval, consent, or child-agent tool
scope validation.

## Config

`~/.roder/config.toml` supports:

```toml
[dynamic_workflows]
enabled = true
trigger_word_enabled = true
auto_with_ultracode = true
max_concurrent_agents = 16
max_agents_per_run = 1000
default_agent_timeout_seconds = 180
default_run_timeout_seconds = 14400
default_checkpoint_bytes = 1048576
max_report_bytes = 65536
workspace_workflows_dir = ".agents/workflows"
user_workflows_dir = "~/.roder/workflows"

[dynamic_workflows.approval]
require_approval = true
allow_always_for_script_and_workspace = true
consent_ttl_seconds = 86400
```

Environment overrides:

- `RODER_DYNAMIC_WORKFLOWS_DISABLED`
- `RODER_DYNAMIC_WORKFLOWS_ENABLED`
- `RODER_DYNAMIC_WORKFLOWS_TRIGGER_WORD`
- `RODER_DYNAMIC_WORKFLOWS_AUTO_WITH_ULTRACODE`
- `RODER_DYNAMIC_WORKFLOWS_MAX_AGENTS`
- `RODER_DYNAMIC_WORKFLOWS_MAX_CONCURRENT_AGENTS`
- `RODER_DYNAMIC_WORKFLOWS_AGENT_TIMEOUT_SECONDS`
- `RODER_DYNAMIC_WORKFLOWS_RUN_TIMEOUT_SECONDS`
- `RODER_DYNAMIC_WORKFLOWS_CHECKPOINT_BYTES`
- `RODER_DYNAMIC_WORKFLOWS_MAX_REPORT_BYTES`
- `RODER_DYNAMIC_WORKFLOWS_WORKSPACE_DIR`
- `RODER_DYNAMIC_WORKFLOWS_USER_DIR`
- `RODER_DYNAMIC_WORKFLOWS_REQUIRE_APPROVAL`
- `RODER_DYNAMIC_WORKFLOWS_ALLOW_ALWAYS_APPROVAL`
- `RODER_DYNAMIC_WORKFLOWS_CONSENT_TTL_SECONDS`
- `RODER_DYNAMIC_WORKFLOWS_LIVE`
- `RODER_DEEP_RESEARCH_LIVE`

Normal tests and eval fixtures stay offline. Live provider or live web-search
checks must be explicitly gated with `RODER_DYNAMIC_WORKFLOWS_LIVE=1` or
`RODER_DEEP_RESEARCH_LIVE=1`.
