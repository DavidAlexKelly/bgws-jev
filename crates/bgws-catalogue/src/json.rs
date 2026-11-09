//! JSON helpers for resolving partial records (§14.1 "defaults per class, so
//! a new platform can be added with a few numbers"): a deep merge where
//! objects merge key by key, arrays and scalars replace, and `null` deletes;
//! and small path accessors the derivation rules use. Keys starting with `$`
//! are metadata (`$source`, `$comment`, `$extends`) and never merge.

use serde_json::{Map, Value};

/// Merge `overlay` onto `base` in place.
pub fn deep_merge(base: &mut Value, overlay: &Value) {
    match (base, overlay) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                if k.starts_with('$') {
                    continue;
                }
                if v.is_null() {
                    b.remove(k);
                    continue;
                }
                match b.get_mut(k) {
                    Some(existing) if existing.is_object() && v.is_object() => deep_merge(existing, v),
                    _ => {
                        b.insert(k.clone(), strip_meta(v));
                    }
                }
            }
        }
        (b, o) => *b = strip_meta(o),
    }
}

/// A copy without `$` metadata keys, at any depth.
pub fn strip_meta(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(m.iter().filter(|(k, _)| !k.starts_with('$')).map(|(k, v)| (k.clone(), strip_meta(v))).collect()),
        Value::Array(a) => Value::Array(a.iter().map(strip_meta).collect()),
        other => other.clone(),
    }
}

/// The value at a dotted path of object keys.
pub fn get<'a>(v: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut cur = v;
    for k in path {
        cur = cur.as_object()?.get(*k)?;
    }
    Some(cur)
}

pub fn get_f64(v: &Value, path: &[&str]) -> Option<f64> {
    get(v, path).and_then(Value::as_f64)
}

pub fn get_str<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    get(v, path).and_then(Value::as_str)
}

/// The object at a path, created (with empty objects on the way) if missing.
/// `None` if something on the way is not an object.
pub fn obj_mut<'a>(v: &'a mut Value, path: &[&str]) -> Option<&'a mut Map<String, Value>> {
    let mut cur = v;
    for k in path {
        let m = cur.as_object_mut()?;
        cur = m.entry(k.to_string()).or_insert_with(|| Value::Object(Map::new()));
    }
    cur.as_object_mut()
}

/// Set `key` in the object at `path` only if it is absent.
pub fn set_default(v: &mut Value, path: &[&str], key: &str, value: Value) {
    if let Some(m) = obj_mut(v, path) {
        m.entry(key.to_string()).or_insert(value);
    }
}

/// Is a number present and non-zero at this path?
pub fn has_num(v: &Value, path: &[&str]) -> bool {
    get_f64(v, path).is_some_and(|x| x != 0.0)
}

/// Visit every number in a JSON tree with its key path (array indices left
/// out, so `sensors[2].rangeMaxM` is `["sensors", "rangeMaxM"]`).
pub fn walk_numbers<'a>(v: &'a Value, path: &mut Vec<&'a str>, f: &mut dyn FnMut(&[&'a str], f64)) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                path.push(k.as_str());
                walk_numbers(x, path, f);
                path.pop();
            }
        }
        Value::Array(a) => {
            for x in a {
                walk_numbers(x, path, f);
            }
        }
        Value::Number(n) => {
            if let Some(x) = n.as_f64() {
                f(path, x);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merges_objects_replaces_arrays_and_null_deletes() {
        let mut a = json!({"a": {"x": 1, "y": 2}, "list": [1, 2], "gone": 5});
        deep_merge(&mut a, &json!({"a": {"y": 3, "$comment": "no"}, "list": [9], "gone": null, "$source": "P1"}));
        assert_eq!(a, json!({"a": {"x": 1, "y": 3}, "list": [9]}));
    }

    #[test]
    fn paths() {
        let mut v = json!({});
        set_default(&mut v, &["mover"], "roadKmh", json!(60));
        set_default(&mut v, &["mover"], "roadKmh", json!(10));
        assert_eq!(get_f64(&v, &["mover", "roadKmh"]), Some(60.0));
        let mut seen = vec![];
        walk_numbers(&json!({"s": [{"r": 1}, {"r": 2}]}), &mut vec![], &mut |p, x| seen.push((p.join("."), x)));
        assert_eq!(seen, vec![("s.r".to_string(), 1.0), ("s.r".to_string(), 2.0)]);
    }
}
