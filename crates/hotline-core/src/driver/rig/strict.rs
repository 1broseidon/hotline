//! Tools on a strict route. Copilot's `/responses` marks every function tool
//! strict, and a strict schema lists every property as required, so a model
//! there has to fill even the ones a tool leaves optional. Given no way to
//! say "not this one" it invents a value: an empty `text` beside the
//! `secret` it meant, a zero where nothing was asked. Here an optional
//! property may be null instead, and a null is taken back out of the call
//! before the tool reads it, so the tool sees the property left out.

use super::*;

/// Whether this model's tools go out strict: Copilot's `/responses`, which
/// Rig takes for the Codex family and this driver for a model offered
/// nowhere else.
pub(super) fn strict_tools(keys: &HashMap<String, ProviderAuth>, model_id: &str) -> bool {
    model_id.split_once('/').is_some_and(|(provider, model)| {
        models::wiring(provider).is_some_and(|wiring| wiring.client == Client::Copilot)
            && (model.to_ascii_lowercase().contains("codex")
                || copilot_responses(keys, provider, model))
    })
}

/// A tool's parameters as a strict route should see them: every property
/// the tool leaves optional may also be null, at any depth.
pub(super) fn nullable_optionals(schema: &mut Value) {
    let Value::Object(object) = schema else {
        return;
    };
    let required: Vec<String> = object
        .get("required")
        .and_then(Value::as_array)
        .map(|names| {
            names
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if let Some(Value::Object(properties)) = object.get_mut("properties") {
        for (name, property) in properties.iter_mut() {
            nullable_optionals(property);
            if !required.contains(name) {
                nullable(property);
            }
        }
    }
    if let Some(items) = object.get_mut("items") {
        nullable_optionals(items);
    }
    for key in ["$defs", "definitions"] {
        if let Some(Value::Object(definitions)) = object.get_mut(key) {
            definitions.values_mut().for_each(nullable_optionals);
        }
    }
    for key in ["anyOf", "oneOf", "allOf"] {
        if let Some(Value::Array(variants)) = object.get_mut(key) {
            variants.iter_mut().for_each(nullable_optionals);
        }
    }
}

/// One property that may also be null: a `null` beside its type (and in
/// its `enum`, which would refuse it otherwise), or a null variant beside
/// a schema that names no type.
fn nullable(property: &mut Value) {
    let Value::Object(object) = property else {
        return;
    };
    let null = Value::String("null".into());
    match object.get_mut("type") {
        Some(Value::String(kind)) if kind != "null" => {
            let kind = Value::String(std::mem::take(kind));
            object.insert("type".into(), Value::Array(vec![kind, null]));
        }
        Some(Value::Array(kinds)) if !kinds.contains(&null) => kinds.push(null),
        Some(_) => {}
        None => {
            if let Some(Value::Array(variants)) = object.get_mut("anyOf") {
                if !variants.iter().any(is_null_schema) {
                    variants.push(serde_json::json!({"type": "null"}));
                }
            } else {
                let inner = Value::Object(std::mem::take(object));
                object.insert(
                    "anyOf".into(),
                    Value::Array(vec![inner, serde_json::json!({"type": "null"})]),
                );
            }
            return;
        }
    }
    if let Some(Value::Array(values)) = object.get_mut("enum")
        && !values.contains(&Value::Null)
    {
        values.push(Value::Null);
    }
}

fn is_null_schema(schema: &Value) -> bool {
    schema.get("type").and_then(Value::as_str) == Some("null")
}

/// A strict call's arguments with every null left out, at any depth, so a
/// tool reads the properties the model filled with null as never given.
pub(super) fn without_nulls(arguments: &Value) -> Value {
    match arguments {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .filter(|(_, value)| !value.is_null())
                .map(|(key, value)| (key.clone(), without_nulls(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(without_nulls).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_optional_property_may_be_null_and_a_required_one_may_not() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["fill", "click"]},
                "text": {"type": "string", "description": "fill only"},
                "secret": {"type": "string"},
                "region": {"type": "array", "items": {"type": "integer"}},
                "mode": {"type": "string", "enum": ["fast", "slow"]},
                "target": {"$ref": "#/$defs/target"},
                "either": {"anyOf": [{"type": "string"}, {"type": "integer"}]},
                "options": {
                    "type": "object",
                    "properties": {
                        "wait": {"type": "boolean"},
                        "selector": {"type": "string"}
                    },
                    "required": ["selector"]
                }
            },
            "required": ["action"]
        });
        nullable_optionals(&mut schema);
        let properties = &schema["properties"];
        assert_eq!(properties["action"]["type"], "string");
        assert_eq!(properties["action"]["enum"], json!(["fill", "click"]));
        assert_eq!(properties["text"]["type"], json!(["string", "null"]));
        assert_eq!(properties["text"]["description"], "fill only");
        assert_eq!(properties["region"]["type"], json!(["array", "null"]));
        assert_eq!(properties["region"]["items"]["type"], "integer");
        assert_eq!(properties["mode"]["enum"], json!(["fast", "slow", null]));
        assert_eq!(
            properties["target"],
            json!({"anyOf": [{"$ref": "#/$defs/target"}, {"type": "null"}]})
        );
        assert_eq!(
            properties["either"]["anyOf"],
            json!([{"type": "string"}, {"type": "integer"}, {"type": "null"}])
        );
        let options = &properties["options"];
        assert_eq!(options["type"], json!(["object", "null"]));
        assert_eq!(
            options["properties"]["wait"]["type"],
            json!(["boolean", "null"])
        );
        assert_eq!(options["properties"]["selector"]["type"], "string");

        // Twice is once: a schema already nullable is left as it is.
        let once = schema.clone();
        nullable_optionals(&mut schema);
        assert_eq!(schema, once);
    }

    #[test]
    fn a_null_the_model_had_to_send_never_reaches_the_tool() {
        let arguments = json!({
            "action": "fill",
            "text": null,
            "secret": "github",
            "region": [0, 0, 10, 10],
            "options": {"wait": null, "selector": "#password"},
            "steps": [{"click": "#go", "text": null}]
        });
        assert_eq!(
            without_nulls(&arguments),
            json!({
                "action": "fill",
                "secret": "github",
                "region": [0, 0, 10, 10],
                "options": {"selector": "#password"},
                "steps": [{"click": "#go"}]
            })
        );
    }

    #[test]
    fn only_copilots_responses_route_is_strict() {
        let scratch = tempfile::tempdir().unwrap();
        let keys = HashMap::from([
            (
                "github-copilot".to_string(),
                ProviderAuth::Login {
                    token_dir: scratch.path().to_path_buf(),
                },
            ),
            (
                "openai".to_string(),
                ProviderAuth::ApiKey("sk-test".to_string()),
            ),
        ]);
        assert!(strict_tools(&keys, "github-copilot/gpt-5.3-codex"));
        assert!(!strict_tools(&keys, "github-copilot/claude-sonnet-5"));
        assert!(!strict_tools(&keys, "openai/gpt-5.3-codex"));
        assert!(!strict_tools(&keys, "no-provider"));
    }
}
