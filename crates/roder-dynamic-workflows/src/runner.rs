use crate::host_api::{RawWorkflowExecution, WorkflowExecution};
use crate::model::{
    WorkflowRunInput, WorkflowRuntimeError, WorkflowRuntimeErrorKind, WorkflowRuntimeOptions,
    WorkflowRuntimeResult,
};
use crate::script::{
    eval_js, install_definition_prelude, new_context, parse_definition_json, preflight_script,
    read_global_string,
};

#[derive(Debug, Clone, Default)]
pub struct WorkflowScriptRuntime {
    options: WorkflowRuntimeOptions,
}

impl WorkflowScriptRuntime {
    pub fn new(options: WorkflowRuntimeOptions) -> Self {
        Self { options }
    }

    pub fn options(&self) -> &WorkflowRuntimeOptions {
        &self.options
    }

    pub fn run(
        &self,
        source: &str,
        input: WorkflowRunInput,
    ) -> WorkflowRuntimeResult<WorkflowExecution> {
        preflight_script(source)?;

        let mut context = new_context(&self.options);
        install_definition_prelude(&mut context)?;
        install_host_prelude(&mut context, &self.options, &input)?;
        eval_js(
            &mut context,
            source,
            WorkflowRuntimeErrorKind::ScriptExecution,
        )?;

        let metadata = read_global_string(&mut context, "globalThis.__roderWorkflowMetadataJson")?
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                WorkflowRuntimeError::new(
                    WorkflowRuntimeErrorKind::MissingDefinition,
                    "script must call workflow.define(metadata, handler)",
                )
            })?;
        let definition = parse_definition_json(&metadata, &self.options)?;
        let run_source = workflow_run_source();
        eval_js(
            &mut context,
            &run_source,
            WorkflowRuntimeErrorKind::ScriptExecution,
        )?;

        for _ in 0..self.options.max_promise_drains {
            context.run_jobs().map_err(|err| {
                crate::script::classify_js_error(
                    err.to_string(),
                    WorkflowRuntimeErrorKind::ScriptExecution,
                )
            })?;
            if let Some(error) =
                read_global_string(&mut context, "globalThis.__roderWorkflowError")?
            {
                return Err(crate::script::classify_js_error(
                    error,
                    WorkflowRuntimeErrorKind::ScriptExecution,
                ));
            }
            if let Some(json) =
                read_global_string(&mut context, "globalThis.__roderWorkflowResultJson")?
            {
                let raw = parse_execution_json(&json)?;
                return Ok(WorkflowExecution {
                    definition,
                    report: raw.report,
                    phases: raw.phases,
                    agent_launches: raw.agent_launches,
                    checkpoints: raw.checkpoints,
                });
            }
        }

        Err(WorkflowRuntimeError::new(
            WorkflowRuntimeErrorKind::ScriptExecution,
            "workflow handler did not settle within the promise drain limit",
        ))
    }
}

fn install_host_prelude(
    context: &mut boa_engine::Context,
    options: &WorkflowRuntimeOptions,
    input: &WorkflowRunInput,
) -> WorkflowRuntimeResult<()> {
    let run_id = serde_json::to_string(&input.run_id).expect("run id serializes");
    let arguments = serde_json::to_string(&input.arguments).map_err(|err| {
        WorkflowRuntimeError::new(
            WorkflowRuntimeErrorKind::InvalidMetadata,
            format!("workflow arguments must be JSON serializable: {err}"),
        )
    })?;
    let limits = serde_json::to_string(&options.limits).expect("limits serialize");
    let checkpoints = serde_json::to_string(&input.checkpoints).map_err(|err| {
        WorkflowRuntimeError::new(
            WorkflowRuntimeErrorKind::InvalidMetadata,
            format!("workflow checkpoints must be JSON serializable: {err}"),
        )
    })?;
    let abort = if input.abort_before_start {
        "true"
    } else {
        "false"
    };
    let max_report_bytes = options.max_report_bytes;

    let prelude = format!(
        r#"
globalThis.__roderRunId = {run_id};
globalThis.__roderArguments = {arguments};
globalThis.__roderLimits = Object.freeze({limits});
globalThis.__roderAbort = {abort};
globalThis.__roderMaxReportBytes = {max_report_bytes};
globalThis.__roderState = {{
  launches: [],
  checkpoints: {{}},
  checkpointRecords: [],
  phases: [],
  currentPhase: null,
  report: null,
  agentCount: 0
}};
for (const checkpoint of {checkpoints}) {{
  globalThis.__roderState.checkpoints[checkpoint.key] = checkpoint.value;
}}

globalThis.__roderCheckAbort = function() {{
  if (globalThis.__roderAbort) {{
    throw new Error("abort:workflow aborted before host call");
  }}
}};

globalThis.__roderLaunchAgent = function(role, descriptor, input, index) {{
  globalThis.__roderCheckAbort();
  const spec = descriptor || {{}};
  if (globalThis.__roderState.agentCount >= globalThis.__roderLimits.maxAgentsPerRun) {{
    throw new Error("limit:maxAgentsPerRun");
  }}
  const launch = {{
    index,
    role,
    lane: spec.lane || role,
    phase: globalThis.__roderState.currentPhase,
    description: spec.description || "",
    prompt: spec.prompt || "",
    model: spec.model || null,
    timeoutSeconds: spec.timeoutSeconds || globalThis.__roderLimits.defaultAgentTimeoutSeconds,
    input: input === undefined ? null : input,
    output: spec.output || `result:${{role}}:${{index}}`
  }};
  globalThis.__roderState.agentCount += 1;
  globalThis.__roderState.launches.push(launch);
  return Object.freeze({{
    agentId: `agent-${{globalThis.__roderState.agentCount}}`,
    role: launch.role,
    lane: launch.lane,
    input: launch.input,
    prompt: launch.prompt,
    output: launch.output
  }});
}};

globalThis.__roderCreateContext = function() {{
  const agents = Object.freeze({{
    run(role, descriptor) {{
      return globalThis.__roderLaunchAgent(role, descriptor, null, 0);
    }},
    map(role, items, mapper) {{
      globalThis.__roderCheckAbort();
      if (!Array.isArray(items)) {{
        throw new Error("ctx.agents.map requires an array of items");
      }}
      if (typeof mapper !== "function") {{
        throw new Error("ctx.agents.map requires a mapper function");
      }}
      return items.map((item, index) => {{
        const descriptor = mapper(item, index) || {{}};
        return globalThis.__roderLaunchAgent(role, descriptor, item, index);
      }});
    }},
    reduce(role, items, mapper, reducer, initial) {{
      if (typeof reducer !== "function") {{
        throw new Error("ctx.agents.reduce requires a reducer function");
      }}
      return agents.map(role, items, mapper).reduce(reducer, initial);
    }}
  }});

  const checkpoint = Object.freeze({{
    save(key, value) {{
      globalThis.__roderCheckAbort();
      const json = JSON.stringify(value);
      if (json.length > globalThis.__roderLimits.defaultCheckpointBytes) {{
        throw new Error("limit:checkpointBytes");
      }}
      const record = {{ key, value, byteCount: json.length }};
      globalThis.__roderState.checkpoints[key] = value;
      globalThis.__roderState.checkpointRecords.push(record);
      return value;
    }},
    read(key) {{
      return Object.prototype.hasOwnProperty.call(globalThis.__roderState.checkpoints, key)
        ? globalThis.__roderState.checkpoints[key]
        : null;
    }}
  }});

  const report = Object.freeze({{
    markdown(value) {{
      globalThis.__roderCheckAbort();
      let text;
      if (Array.isArray(value)) {{
        text = value.map((item) => typeof item === "string" ? item : (item.output || JSON.stringify(item))).join("\n");
      }} else {{
        text = String(value);
      }}
      if (text.length > globalThis.__roderMaxReportBytes) {{
        throw new Error("limit:reportBytes");
      }}
      globalThis.__roderState.report = text;
      return text;
    }}
  }});

  return Object.freeze({{
    run: Object.freeze({{ id: globalThis.__roderRunId, arguments: globalThis.__roderArguments }}),
    phase: Object.freeze({{ start(name) {{ const phase = String(name); globalThis.__roderState.currentPhase = phase; globalThis.__roderState.phases.push(phase); return phase; }} }}),
    agents,
    results: Object.freeze({{
      all() {{ return globalThis.__roderState.launches.slice(); }},
      vote(items, selector) {{
        if (!Array.isArray(items)) {{
          throw new Error("ctx.results.vote requires an array of items");
        }}
        const choose = typeof selector === "function" ? selector : (item) => item;
        const counts = {{}};
        for (const item of items) {{
          const key = String(choose(item));
          counts[key] = (counts[key] || 0) + 1;
        }}
        let winner = null;
        let winnerCount = -1;
        for (const [key, count] of Object.entries(counts)) {{
          if (count > winnerCount) {{
            winner = key;
            winnerCount = count;
          }}
        }}
        return {{ winner, counts }};
      }}
    }}),
    checkpoint,
    report,
    limits: globalThis.__roderLimits,
    abortSignal: Object.freeze({{ get aborted() {{ return globalThis.__roderAbort; }} }})
  }});
}};
"#
    );

    eval_js(context, &prelude, WorkflowRuntimeErrorKind::ScriptExecution)
}

fn workflow_run_source() -> String {
    r#"
(async () => {
  try {
    if (typeof globalThis.__roderWorkflowHandler !== "function") {
      throw new Error("workflow.define must install a handler before execution");
    }
    const ctx = globalThis.__roderCreateContext();
    const result = await globalThis.__roderWorkflowHandler(ctx);
    const report = globalThis.__roderState.report !== null
      ? globalThis.__roderState.report
      : (typeof result === "string" ? result : JSON.stringify(result));
    globalThis.__roderWorkflowResultJson = JSON.stringify({
      report,
      phases: globalThis.__roderState.phases,
      agentLaunches: globalThis.__roderState.launches,
      checkpoints: globalThis.__roderState.checkpointRecords
    });
  } catch (error) {
    globalThis.__roderWorkflowError = String(error && error.message ? error.message : error);
  }
})();
"#
    .to_string()
}

fn parse_execution_json(json: &str) -> WorkflowRuntimeResult<RawWorkflowExecution> {
    serde_json::from_str(json).map_err(|err| {
        WorkflowRuntimeError::new(
            WorkflowRuntimeErrorKind::ScriptExecution,
            format!("workflow produced invalid execution JSON: {err}"),
        )
    })
}
