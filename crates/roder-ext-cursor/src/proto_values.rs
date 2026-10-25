use super::*;

/// Decode the google.protobuf.Value used by Cursor's MCP argument maps.
/// Bound nesting independently of Connect frame size.
pub(super) fn decode_protobuf_value(bytes: &[u8]) -> Value {
    fn decode(bytes: &[u8], depth: usize) -> Value {
        if depth > 32 {
            return Value::Null;
        }
        for field in decode_fields_safe(bytes) {
            match (field.no, field.value) {
                (1, ProtoValue::Varint(_)) => return Value::Null,
                (2, ProtoValue::Fixed64(bits)) => {
                    let number = f64::from_bits(bits);
                    // protobuf.Value has only doubles. Recover exact integral values so
                    // MCP integer schemas and typed tool arguments accept values such as 8.
                    // Beyond the safe-integer range, keep the floating representation.
                    if number.is_finite()
                        && number.fract() == 0.0
                        && number.abs() <= 9_007_199_254_740_991.0
                    {
                        return Value::from(number as i64);
                    }
                    return serde_json::Number::from_f64(number)
                        .map(Value::Number)
                        .unwrap_or(Value::Null);
                }
                (3, ProtoValue::Bytes(text)) => {
                    return Value::String(String::from_utf8_lossy(&text).into_owned());
                }
                (4, ProtoValue::Varint(value)) => return Value::Bool(value != 0),
                (5, ProtoValue::Bytes(object)) => {
                    let mut result = serde_json::Map::new();
                    for field in decode_fields_safe(&object) {
                        if let (1, ProtoValue::Bytes(entry)) = (field.no, field.value)
                            && let (Some(key), Some(value)) =
                                (scalar_string(&entry, 1), submessage(&entry, 2))
                        {
                            result.insert(key, decode(&value, depth + 1));
                        }
                    }
                    return Value::Object(result);
                }
                (6, ProtoValue::Bytes(list)) => {
                    return Value::Array(
                        decode_fields_safe(&list)
                            .into_iter()
                            .filter_map(|field| {
                                if let (1, ProtoValue::Bytes(value)) = (field.no, field.value) {
                                    Some(decode(&value, depth + 1))
                                } else {
                                    None
                                }
                            })
                            .collect(),
                    );
                }
                _ => {}
            }
        }
        Value::Null
    }
    decode(bytes, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trips_nested_mcp_arguments() {
        let value =
            serde_json::json!({"null":null,"bool":true,"number":1.25,"list":["x",{"n":-7.5}]});
        assert_eq!(decode_protobuf_value(&encode_protobuf_value(&value)), value);
    }

    #[test]
    fn mcp_timer_arguments_retain_integer_types() {
        let arguments = serde_json::json!({"topic":"proof:once", "prompt":"remind", "delaySeconds":8, "intervalSeconds":null, "maxDeliveries":1, "ttlSeconds":120});
        let decoded = decode_protobuf_value(&encode_protobuf_value(&arguments));
        assert_eq!(decoded["delaySeconds"].as_u64(), Some(8));
        let request: roder_api::thread_events::ThreadEventSubscribeRequest =
            serde_json::from_value(decoded).unwrap();
        assert_eq!(request.delay_seconds, Some(8));
        assert_eq!(request.max_deliveries, 1);
    }

    #[test]
    fn fractional_and_unsafe_numbers_are_not_coerced_to_integers() {
        for number in [1.25, -7.5, 9_007_199_254_740_992.0] {
            let value = Value::from(number);
            let decoded = decode_protobuf_value(&encode_protobuf_value(&value));
            assert_eq!(decoded.as_f64(), Some(number));
            assert!(!decoded.is_i64());
        }
        assert_eq!(
            decode_protobuf_value(&encode_protobuf_value(&Value::from(-7))),
            Value::from(-7)
        );
    }
}
