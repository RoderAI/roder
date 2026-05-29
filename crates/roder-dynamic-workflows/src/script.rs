use boa_engine::{Context, Source};

use crate::model::{
    RawWorkflowDefinition, WorkflowDefinition, WorkflowRuntimeError, WorkflowRuntimeErrorKind,
    WorkflowRuntimeOptions, WorkflowRuntimeResult,
};

const DENIED_AMBIENT_PATTERNS: &[(&str, &str)] = &[
    (
        "import(",
        "dynamic module loading is not available in workflow scripts",
    ),
    (
        "import ",
        "module loading is not available in workflow scripts",
    ),
    (
        "require(",
        "CommonJS loading is not available in workflow scripts",
    ),
    ("eval(", "eval is not available in workflow scripts"),
    (
        "Function(",
        "dynamic function construction is not available in workflow scripts",
    ),
    (
        "process.",
        "ambient process access is not available in workflow scripts",
    ),
    (
        "process[",
        "ambient process access is not available in workflow scripts",
    ),
    (
        "Deno.",
        "ambient runtime access is not available in workflow scripts",
    ),
    (
        "Bun.",
        "ambient runtime access is not available in workflow scripts",
    ),
    (
        "fetch(",
        "network access is not available in workflow scripts",
    ),
    (
        "XMLHttpRequest",
        "network access is not available in workflow scripts",
    ),
    (
        "WebSocket",
        "network access is not available in workflow scripts",
    ),
    (
        "setTimeout(",
        "ambient timers are not available in workflow scripts",
    ),
    (
        "setInterval(",
        "ambient timers are not available in workflow scripts",
    ),
];

pub fn parse_workflow_definition(
    source: &str,
    options: &WorkflowRuntimeOptions,
) -> WorkflowRuntimeResult<WorkflowDefinition> {
    preflight_script(source)?;
    let mut context = new_context(options);
    install_definition_prelude(&mut context)?;
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
    parse_definition_json(&metadata, options)
}

pub(crate) fn preflight_script(source: &str) -> WorkflowRuntimeResult<()> {
    for (pattern, message) in DENIED_AMBIENT_PATTERNS {
        if source.contains(pattern) {
            return Err(WorkflowRuntimeError::new(
                WorkflowRuntimeErrorKind::DeniedAmbientApi,
                *message,
            ));
        }
    }
    Ok(())
}

pub(crate) fn new_context(options: &WorkflowRuntimeOptions) -> Context {
    let mut context = Context::default();
    context
        .runtime_limits_mut()
        .set_loop_iteration_limit(options.max_loop_iterations);
    context.runtime_limits_mut().set_recursion_limit(128);
    context
}

pub(crate) fn install_definition_prelude(context: &mut Context) -> WorkflowRuntimeResult<()> {
    eval_js(
        context,
        r#"
Object.defineProperty(globalThis, "workflow", {
  value: Object.freeze({
    define(metadata, handler) {
      if (typeof handler !== "function") {
        throw new Error("workflow.define requires a function handler");
      }
      globalThis.__roderWorkflowMetadataJson = JSON.stringify(metadata || {});
      globalThis.__roderWorkflowHandler = handler;
    }
  }),
  writable: false,
  configurable: false
});
"#,
        WorkflowRuntimeErrorKind::ScriptExecution,
    )
}

pub(crate) fn eval_js(
    context: &mut Context,
    source: &str,
    kind: WorkflowRuntimeErrorKind,
) -> WorkflowRuntimeResult<()> {
    context
        .eval(Source::from_bytes(source))
        .map(|_| ())
        .map_err(|err| classify_js_error(err.to_string(), kind))
}

pub(crate) fn read_global_string(
    context: &mut Context,
    expression: &str,
) -> WorkflowRuntimeResult<Option<String>> {
    let value = context
        .eval(Source::from_bytes(expression))
        .map_err(|err| {
            classify_js_error(err.to_string(), WorkflowRuntimeErrorKind::ScriptExecution)
        })?;
    if value.is_null_or_undefined() {
        return Ok(None);
    }
    let text = value
        .to_string(context)
        .map_err(|err| {
            classify_js_error(err.to_string(), WorkflowRuntimeErrorKind::ScriptExecution)
        })?
        .to_std_string_escaped();
    Ok(Some(text))
}

pub(crate) fn parse_definition_json(
    metadata: &str,
    options: &WorkflowRuntimeOptions,
) -> WorkflowRuntimeResult<WorkflowDefinition> {
    let raw: RawWorkflowDefinition = serde_json::from_str(metadata).map_err(|err| {
        WorkflowRuntimeError::new(
            WorkflowRuntimeErrorKind::InvalidMetadata,
            format!("invalid workflow metadata: {err}"),
        )
    })?;
    raw.into_definition(&options.limits)
}

pub(crate) fn classify_js_error(
    message: String,
    default_kind: WorkflowRuntimeErrorKind,
) -> WorkflowRuntimeError {
    if message.contains("loop iteration limit")
        || message.contains("limit:")
        || message.contains("Maximum call stack")
    {
        return WorkflowRuntimeError::new(WorkflowRuntimeErrorKind::LimitExceeded, message);
    }
    if message.contains("abort:") {
        return WorkflowRuntimeError::new(WorkflowRuntimeErrorKind::Aborted, message);
    }
    WorkflowRuntimeError::new(default_kind, message)
}
