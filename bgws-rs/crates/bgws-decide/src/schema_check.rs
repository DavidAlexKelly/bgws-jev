//! A small JSON Schema checker for plans (RUST_ENGINE.md §10.3: "parsing is a
//! validation step, not guesswork"). It covers what the plan schemas use:
//! `type` (one or a list), `required`, `properties`, `additionalProperties:
//! false`, `items`, `enum`, `minimum`/`maximum`, `minItems`/`maxItems`.

use serde_json::Value;

fn type_ok(v: &Value, t: &str) -> bool {
    match t {
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "number" => v.is_number(),
        "integer" => v.as_f64().is_some_and(|x| x.fract() == 0.0),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        _ => true,
    }
}

/// Every place `value` breaks `schema`, as "path: problem". Empty = valid.
pub fn check(schema: &Value, value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    walk(schema, value, "$", &mut out);
    out
}

fn walk(s: &Value, v: &Value, at: &str, out: &mut Vec<String>) {
    let Some(s) = s.as_object() else { return };
    if let Some(t) = s.get("type") {
        let types: Vec<&str> = match t {
            Value::String(t) => vec![t.as_str()],
            Value::Array(ts) => ts.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        if !types.is_empty() && !types.iter().any(|t| type_ok(v, t)) {
            out.push(format!("{at}: expected {}", types.join(" or ")));
            return;
        }
    }
    if let Some(Value::Array(allowed)) = s.get("enum") {
        if !allowed.contains(v) {
            out.push(format!("{at}: {v} is not one of the allowed values"));
        }
    }
    if let Some(x) = v.as_f64() {
        if let Some(min) = s.get("minimum").and_then(Value::as_f64) {
            if x < min {
                out.push(format!("{at}: {x} is below {min}"));
            }
        }
        if let Some(max) = s.get("maximum").and_then(Value::as_f64) {
            if x > max {
                out.push(format!("{at}: {x} is above {max}"));
            }
        }
    }
    if let Some(obj) = v.as_object() {
        for r in s.get("required").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            if !obj.contains_key(r) {
                out.push(format!("{at}: missing \"{r}\""));
            }
        }
        let props = s.get("properties").and_then(Value::as_object);
        for (k, val) in obj {
            match props.and_then(|p| p.get(k)) {
                Some(ps) => walk(ps, val, &format!("{at}.{k}"), out),
                None => match s.get("additionalProperties") {
                    Some(Value::Bool(false)) => out.push(format!("{at}: unexpected \"{k}\"")),
                    Some(extra @ Value::Object(_)) => walk(extra, val, &format!("{at}.{k}"), out),
                    _ => {}
                },
            }
        }
    }
    if let Some(items) = v.as_array() {
        if let Some(min) = s.get("minItems").and_then(Value::as_u64) {
            if (items.len() as u64) < min {
                out.push(format!("{at}: fewer than {min} items"));
            }
        }
        if let Some(max) = s.get("maxItems").and_then(Value::as_u64) {
            if (items.len() as u64) > max {
                out.push(format!("{at}: more than {max} items"));
            }
        }
        if let Some(is) = s.get("items") {
            for (i, it) in items.iter().enumerate() {
                walk(is, it, &format!("{at}[{i}]"), out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn checks_types_required_enums_and_extras() {
        let s = json!({"type": "object", "required": ["a"], "additionalProperties": false,
            "properties": {"a": {"type": "array", "items": {"type": "string", "enum": ["x", "y"]}, "minItems": 1}, "n": {"type": "number", "minimum": 0}}});
        assert!(check(&s, &json!({"a": ["x"]})).is_empty());
        let e = check(&s, &json!({"a": ["z"], "n": -1, "b": 1}));
        assert_eq!(e.len(), 3, "{e:?}");
        assert_eq!(check(&s, &json!({}))[0], "$: missing \"a\"");
        assert_eq!(check(&s, &json!([]))[0], "$: expected object");
    }
}
