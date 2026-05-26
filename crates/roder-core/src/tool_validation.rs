use roder_api::ToolSpec;
use roder_api::events::{
    RoderEvent, ThreadId, ToolCallValidationFailureClass, ToolCallValidationRecorded,
    ToolCallValidationRepairStatus, TurnId,
};
use roder_api::transcript::{ToolResultRecord, tool_display_payload};
use serde_json::Value;
use time::OffsetDateTime;

use crate::runtime::Runtime;

#[derive(Debug, Clone)]
pub(crate) struct ToolValidationError {
    pub(crate) failure_class: ToolCallValidationFailureClass,
    pub(crate) repair_status: ToolCallValidationRepairStatus,
    pub(crate) message: String,
}

pub(crate) async fn validate_tool_call_arguments(
    raw_arguments: &str,
    spec: &ToolSpec,
    thread_id: &ThreadId,
    turn_id: &TurnId,
    tool_id: &str,
    runtime: &Runtime,
) -> Result<Value, ToolValidationError> {
    let mut arguments: Value = match serde_json::from_str(raw_arguments) {
        Ok(value) => value,
        Err(err) => {
            let error = ToolValidationError {
                failure_class: ToolCallValidationFailureClass::InvalidJson,
                repair_status: ToolCallValidationRepairStatus::NotNeeded,
                message: format!("tool arguments must be valid JSON: {err}"),
            };
            emit_validation_error(runtime, thread_id, turn_id, tool_id, spec, &error).await;
            return Err(error);
        }
    };

    if spec.parameters.get("type").and_then(Value::as_str) == Some("object")
        && let Value::String(inner) = &arguments
    {
        match serde_json::from_str::<Value>(inner) {
            Ok(value) if value.is_object() => {
                arguments = value;
                emit_tool_validation_recorded(
                    runtime,
                    thread_id,
                    turn_id,
                    tool_id,
                    &spec.name,
                    ToolCallValidationFailureClass::SchemaRepairApplied,
                    ToolCallValidationRepairStatus::Applied,
                    "repaired stringified JSON object tool arguments".to_string(),
                )
                .await;
            }
            _ => {
                let error = ToolValidationError {
                    failure_class: ToolCallValidationFailureClass::SchemaRepairRejected,
                    repair_status: ToolCallValidationRepairStatus::Rejected,
                    message: "tool arguments were a string, but did not contain a JSON object"
                        .to_string(),
                };
                emit_validation_error(runtime, thread_id, turn_id, tool_id, spec, &error).await;
                return Err(error);
            }
        }
    }

    if let Err(error) = validate_value_against_schema(&arguments, &spec.parameters) {
        emit_validation_error(runtime, thread_id, turn_id, tool_id, spec, &error).await;
        return Err(error);
    }

    Ok(arguments)
}

pub(crate) async fn emit_tool_validation_recorded(
    runtime: &Runtime,
    thread_id: &ThreadId,
    turn_id: &TurnId,
    tool_id: &str,
    tool_name: &str,
    failure_class: ToolCallValidationFailureClass,
    repair_status: ToolCallValidationRepairStatus,
    message: String,
) {
    runtime
        .emit(RoderEvent::ToolCallValidationRecorded(
            ToolCallValidationRecorded {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                tool_id: tool_id.to_string(),
                tool_name: tool_name.to_string(),
                failure_class,
                repair_status,
                message,
                timestamp: OffsetDateTime::now_utc(),
            },
        ))
        .await;
}

pub(crate) fn validation_error_tool_result(
    tool_id: &str,
    tool_name: &str,
    parsed_args: &Value,
    error: ToolValidationError,
) -> ToolResultRecord {
    ToolResultRecord {
        id: tool_id.to_string(),
        name: Some(tool_name.to_string()),
        result: format!("invalid tool call arguments: {}", error.message),
        display_payload: tool_display_payload(Some(tool_name), Some(parsed_args), None),
        is_error: true,
    }
}

async fn emit_validation_error(
    runtime: &Runtime,
    thread_id: &ThreadId,
    turn_id: &TurnId,
    tool_id: &str,
    spec: &ToolSpec,
    error: &ToolValidationError,
) {
    emit_tool_validation_recorded(
        runtime,
        thread_id,
        turn_id,
        tool_id,
        &spec.name,
        error.failure_class.clone(),
        error.repair_status.clone(),
        error.message.clone(),
    )
    .await;
}

fn validate_value_against_schema(value: &Value, schema: &Value) -> Result<(), ToolValidationError> {
    let Some(schema_type) = schema.get("type").and_then(Value::as_str) else {
        return Ok(());
    };
    if !value_matches_type(value, schema_type) {
        return Err(ToolValidationError {
            failure_class: ToolCallValidationFailureClass::WrongType,
            repair_status: ToolCallValidationRepairStatus::NotNeeded,
            message: format!(
                "tool arguments expected {schema_type}, got {}",
                value_kind(value)
            ),
        });
    }

    if schema_type == "object" {
        validate_object_against_schema(value, schema)?;
    }
    if schema_type == "array"
        && let Some(item_schema) = schema.get("items")
        && let Some(items) = value.as_array()
    {
        for item in items {
            validate_value_against_schema(item, item_schema)?;
        }
    }
    Ok(())
}

fn validate_object_against_schema(
    value: &Value,
    schema: &Value,
) -> Result<(), ToolValidationError> {
    let Some(object) = value.as_object() else {
        return Err(ToolValidationError {
            failure_class: ToolCallValidationFailureClass::WrongType,
            repair_status: ToolCallValidationRepairStatus::NotNeeded,
            message: format!("tool arguments expected object, got {}", value_kind(value)),
        });
    };
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for name in required.iter().filter_map(Value::as_str) {
            let Some(property) = object.get(name) else {
                return Err(ToolValidationError {
                    failure_class: ToolCallValidationFailureClass::MissingRequired,
                    repair_status: ToolCallValidationRepairStatus::NotNeeded,
                    message: format!("missing required tool argument `{name}`"),
                });
            };
            if properties
                .get(name)
                .and_then(|schema| schema.get("type"))
                .and_then(Value::as_str)
                == Some("string")
                && property
                    .as_str()
                    .is_some_and(|value| value.trim().is_empty())
            {
                return Err(ToolValidationError {
                    failure_class: ToolCallValidationFailureClass::EmptyRequiredString,
                    repair_status: ToolCallValidationRepairStatus::NotNeeded,
                    message: format!("required string tool argument `{name}` must not be empty"),
                });
            }
        }
    }

    if schema.get("additionalProperties").and_then(Value::as_bool) == Some(false) {
        for name in object.keys() {
            if !properties.contains_key(name) {
                return Err(ToolValidationError {
                    failure_class: ToolCallValidationFailureClass::UnexpectedProperty,
                    repair_status: ToolCallValidationRepairStatus::NotNeeded,
                    message: format!("unexpected tool argument `{name}`"),
                });
            }
        }
    }

    for (name, property_schema) in properties {
        if let Some(property) = object.get(&name) {
            validate_value_against_schema(property, &property_schema)?;
        }
    }
    Ok(())
}

fn value_matches_type(value: &Value, schema_type: &str) -> bool {
    match schema_type {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => true,
    }
}

fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
