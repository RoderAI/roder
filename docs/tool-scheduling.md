# Tool execution scheduling

`ToolExecutor::execution_mode` describes a scheduling contract, not permission:

- `Exclusive` is the default for unknown tools, mutation, and execution.
- `SharedRead` is an explicit promise that the operation is an independent,
  side-effect-free read. Shared reads may run concurrently and during sampling.
- `Coordinator` is reserved for audited delegators that do not access shared
  resources themselves. They must propagate the inherited execution barrier to
  concrete child operations, which acquire it with their own execution modes.

Validation, approvals, policy contributors, tool allowlists, and workspace
authorization are unchanged. A read-only scheduling hint cannot authorize a tool.

The runtime executes adjacent shared reads together, preserving response order
across exclusive operations. A fair runtime-wide read/write barrier also protects
overlapping streamed and provider-driven executions. This is deliberately more
conservative than per-path locking: tools may share state outside the filesystem.
Agent and event coordinators do not hold this barrier while awaiting child work;
the concrete child operations use their normal execution paths. Task and swarm
dispatchers also propagate the same barrier to their in-process child tools,
preventing both recursive-lock deadlocks and unguarded child mutations.

During streaming, the first non-eager call closes the eager window for the rest
of the response, so a later read cannot overtake an earlier mutation. Already
started reads finish before normal dispatch begins. Aborted tool tasks release
their local scheduling guards.

## Remote workspaces

Shared-read metadata does not weaken remote compound-operation fencing. The
existing per-session mutex and provider-authoritative execution lease continue
to cover reads and writes, including read/modify/write operations. Remote shared
read leases would require explicit provider support; no provider is assumed to
offer that support.

## Trusted MCP read-only tools

MCP annotations are untrusted server hints. To opt a specific audited tool into
shared scheduling, configure its exact remote name:

```json
{
  "mcpServers": {
    "files": {
      "url": "https://example.invalid/mcp",
      "trusted_read_only_tools": ["list_files", "read_file"]
    }
  }
}
```

The discovered tool must also advertise `annotations.readOnlyHint: true` and
must not advertise `destructiveHint: true`. Missing annotations, contradictory
hints, unlisted tools, and unknown tools remain exclusive. Names are exact;
wildcards do not grant trust. Authentication and approval policy remain unchanged.
