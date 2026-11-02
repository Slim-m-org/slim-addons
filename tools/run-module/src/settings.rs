//! The `settings` a run carries: the manifest's defaults with `--setting key=value`
//! laid over them, each checked against the setting's declared type and range.

use serde_json::{Map, Value};

/// `None` when there is nothing to send, as slim leaves `settings` out.
pub fn resolve(manifest: Option<&Value>, given: &[(String, String)]) -> Result<Option<Value>, String> {
    let declared = manifest.and_then(|m| m["settings"].as_array()).cloned().unwrap_or_default();
    let mut values = Map::new();
    for s in declared.iter().filter(|s| s["type"] != "secret") {
        if let (Some(key), Some(default)) = (s["key"].as_str(), s.get("default")) {
            values.insert(key.to_owned(), default.clone());
        }
    }
    for (key, raw) in given {
        let setting = declared.iter().find(|s| s["key"] == key.as_str());
        let value = match setting {
            Some(s) => typed(s, raw).map_err(|e| format!("--setting {key}: {e}"))?,
            None if manifest.is_some() => return Err(format!("--setting {key}: the manifest declares no such setting")),
            None => serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.clone())),
        };
        values.insert(key.clone(), value);
    }
    Ok((!values.is_empty()).then_some(Value::Object(values)))
}

fn typed(setting: &Value, raw: &str) -> Result<Value, String> {
    match setting["type"].as_str().unwrap_or_default() {
        "bool" => raw.parse::<bool>().map(Value::Bool).map_err(|_| "needs true or false".to_owned()),
        "int" => {
            let n: i64 = raw.parse().map_err(|_| "needs a whole number".to_owned())?;
            let (min, max) = (setting["min"].as_i64(), setting["max"].as_i64());
            if min.is_some_and(|m| n < m) || max.is_some_and(|m| n > m) {
                return Err(format!("must be {} to {}", min.unwrap_or(i64::MIN), max.unwrap_or(i64::MAX)));
            }
            Ok(Value::from(n))
        }
        "choice" => {
            let known = setting["choices"].as_array().is_some_and(|c| c.iter().any(|c| c["value"] == raw));
            if known { Ok(Value::String(raw.to_owned())) } else { Err("is not one of its choices".to_owned()) }
        }
        "secret" => Err("is a secret, which a module never reads".to_owned()),
        _ => Ok(Value::String(raw.to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn manifest() -> Value {
        json!({"settings": [
            {"key": "rate", "type": "int", "default": 5, "min": 0, "max": 100},
            {"key": "curve", "type": "choice", "default": "standard", "choices": [{"value": "standard"}, {"value": "steep"}]},
            {"key": "api_key", "type": "secret"},
            {"key": "solo", "type": "bool"}
        ]})
    }

    fn set(pairs: &[(&str, &str)]) -> Result<Option<Value>, String> {
        let given: Vec<(String, String)> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        resolve(Some(&manifest()), &given)
    }

    #[test]
    fn defaults_fill_in_and_a_given_value_wins() {
        assert_eq!(set(&[]).unwrap(), Some(json!({"rate": 5, "curve": "standard"})));
        assert_eq!(set(&[("rate", "9"), ("solo", "true")]).unwrap(), Some(json!({"rate": 9, "curve": "standard", "solo": true})));
    }

    #[test]
    fn a_value_outside_its_declared_shape_is_refused() {
        assert!(set(&[("rate", "101")]).unwrap_err().contains("0 to 100"));
        assert!(set(&[("curve", "flat")]).is_err());
        assert!(set(&[("api_key", "x")]).unwrap_err().contains("secret"));
        assert!(set(&[("nope", "1")]).is_err());
        assert_eq!(resolve(None, &[]).unwrap(), None);
    }
}
