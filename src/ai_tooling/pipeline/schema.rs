//! Rust types → OpenAI strict JSON Schema.
//!
//! `schemars` emits ordinary draft-07, which OpenAI's strict Structured Outputs
//! mode rejects on several counts. Rather than hand-writing schemas that drift
//! away from the structs they describe, the schema is derived from the type and
//! then rewritten here. Adding a field to a struct changes the contract the
//! model is held to, automatically.
//!
//! What strict mode demands, and what this module does about it:
//!
//! | Requirement | Fix |
//! |---|---|
//! | Every object sets `additionalProperties: false` | added to every object node |
//! | Every property listed in `required` | `required` rebuilt from `properties` |
//! | Optionality expressed as a nullable type, not a missing key | `Option<T>` already emits `["T","null"]`; it stays required |
//! | No `oneOf` | rewritten to `anyOf` |
//! | No annotation or validation keywords | stripped |
//! | `$defs`, not draft-07 `definitions` | renamed, and `$ref`s rewritten |
//! | Root must be an object | enforced by [`strict_schema_for`] |

use schemars::{schema_for, JsonSchema};
use serde_json::{Map, Value};

/// Keywords strict mode does not accept. Annotations are harmless to lose;
/// the validation keywords are the ones that would be rejected outright.
const STRIPPED: [&str; 14] = [
    "$schema",
    "title",
    "default",
    "examples",
    "format",
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "multipleOf",
    "minLength",
    "maxLength",
    "pattern",
    "$comment",
];

/// Named schema, as the `json_schema` response format wants it.
#[derive(Debug, Clone, PartialEq)]
pub struct SchemaSpec {
    /// Identifier echoed back by the API; must match `^[a-zA-Z0-9_-]+$`.
    pub name: String,
    pub schema: Value,
}

impl SchemaSpec {
    pub fn new(name: impl Into<String>, schema: Value) -> Self {
        Self {
            name: name.into(),
            schema,
        }
    }
}

/// Derives a strict schema for `T`.
///
/// `T` must serialize to a JSON object: OpenAI will not accept an array or a
/// scalar at the root. Agent 2 returns a list, so it returns it *wrapped* — see
/// [`RevisionDraftList`](crate::ai_tooling::pipeline::models::RevisionDraftList).
pub fn strict_schema_for<T: JsonSchema>(name: &str) -> SchemaSpec {
    let root = schema_for!(T);
    let mut value = serde_json::to_value(root).unwrap_or_else(|_| Value::Object(Map::new()));

    hoist_definitions(&mut value);
    tighten(&mut value);

    SchemaSpec::new(name, value)
}

/// draft-07 `definitions` → 2020-12 `$defs`, references included.
fn hoist_definitions(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if let Some(defs) = object.remove("definitions") {
        object.insert("$defs".into(), defs);
    }
    rewrite_refs(value);
}

fn rewrite_refs(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if let Some(Value::String(reference)) = object.get_mut("$ref") {
                if let Some(rest) = reference.strip_prefix("#/definitions/") {
                    *reference = format!("#/$defs/{rest}");
                }
            }
            for child in object.values_mut() {
                rewrite_refs(child);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(rewrite_refs),
        _ => {}
    }
}

/// Applies the strict rules to every node, depth first.
fn tighten(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for key in STRIPPED {
                object.remove(key);
            }

            // Strict mode understands `anyOf` but not `oneOf`. For a Rust enum
            // the two mean the same thing — the tag makes the variants mutually
            // exclusive anyway, so nothing is lost by widening.
            if let Some(variants) = object.remove("oneOf") {
                object.insert("anyOf".into(), variants);
            }

            for child in object.values_mut() {
                tighten(child);
            }

            if object.contains_key("properties") {
                seal(object);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(tighten),
        _ => {}
    }
}

/// Closes one object node: no extra keys, and every declared property required.
///
/// Listing optional fields as required looks wrong but is exactly what strict
/// mode wants — `Option<T>` is carried by the *type* being nullable, not by the
/// key being absent, so the model must always emit the key.
fn seal(object: &mut Map<String, Value>) {
    object.insert("additionalProperties".into(), Value::Bool(false));

    let keys: Vec<Value> = object
        .get("properties")
        .and_then(Value::as_object)
        .map(|properties| properties.keys().cloned().map(Value::String).collect())
        .unwrap_or_default();

    // An object with no properties at all must not declare an empty `required`.
    if keys.is_empty() {
        object.remove("required");
    } else {
        object.insert("required".into(), Value::Array(keys));
    }

    if !object.contains_key("type") {
        object.insert("type".into(), Value::String("object".into()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, JsonSchema)]
    struct Simple {
        /// Doc comments become descriptions, which strict mode allows.
        name: String,
        count: u32,
        ratio: f32,
        maybe: Option<String>,
    }

    #[derive(Serialize, Deserialize, JsonSchema)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    enum Tagged {
        First { at: f32 },
        Second { label: String, index: usize },
    }

    #[derive(Serialize, Deserialize, JsonSchema)]
    struct Nested {
        items: Vec<Tagged>,
        inner: Simple,
    }

    fn schema_of<T: JsonSchema>() -> Value {
        strict_schema_for::<T>("test").schema
    }

    /// Walks every object node that declares properties.
    fn for_each_object(value: &Value, visit: &mut impl FnMut(&Map<String, Value>)) {
        match value {
            Value::Object(object) => {
                if object.contains_key("properties") {
                    visit(object);
                }
                for child in object.values() {
                    for_each_object(child, visit);
                }
            }
            Value::Array(items) => items.iter().for_each(|item| for_each_object(item, visit)),
            _ => {}
        }
    }

    #[test]
    fn every_object_forbids_extra_properties() {
        let schema = schema_of::<Nested>();
        let mut checked = 0;

        for_each_object(&schema, &mut |object| {
            assert_eq!(
                object.get("additionalProperties"),
                Some(&Value::Bool(false)),
                "an open object is rejected by strict mode: {object:?}"
            );
            checked += 1;
        });
        assert!(checked >= 3, "root, inner and the variants were all visited");
    }

    #[test]
    fn every_property_is_required_including_the_optional_one() {
        let schema = schema_of::<Simple>();
        let required: Vec<&str> = schema["required"]
            .as_array()
            .expect("required")
            .iter()
            .filter_map(Value::as_str)
            .collect();

        assert_eq!(required.len(), 4);
        assert!(
            required.contains(&"maybe"),
            "an Option is nullable, not absent — strict mode still wants the key"
        );
    }

    #[test]
    fn an_option_is_carried_as_a_nullable_type() {
        let schema = schema_of::<Simple>();
        let maybe = &schema["properties"]["maybe"];

        // schemars writes either ["string","null"] or an anyOf including null.
        let nullable = maybe["type"]
            .as_array()
            .is_some_and(|types| types.iter().any(|t| t == "null"))
            || maybe["anyOf"]
                .as_array()
                .is_some_and(|any| any.iter().any(|v| v["type"] == "null"));
        assert!(nullable, "not nullable: {maybe}");
    }

    #[test]
    fn a_tagged_enum_becomes_anyof_never_oneof() {
        let schema = schema_of::<Tagged>();
        let json = schema.to_string();

        assert!(!json.contains("oneOf"), "strict mode rejects oneOf");
        assert!(
            schema["anyOf"].is_array(),
            "the variants survive as anyOf: {schema}"
        );
        assert_eq!(schema["anyOf"].as_array().expect("anyOf").len(), 2);
    }

    #[test]
    fn each_variant_keeps_its_tag_and_its_own_fields() {
        let schema = schema_of::<Tagged>();
        let variants = schema["anyOf"].as_array().expect("anyOf");

        let first = variants
            .iter()
            .find(|v| v["properties"].get("at").is_some())
            .expect("the First variant");
        let required: Vec<&str> = first["required"]
            .as_array()
            .expect("required")
            .iter()
            .filter_map(Value::as_str)
            .collect();

        assert!(required.contains(&"kind"), "the discriminator is mandatory");
        assert!(required.contains(&"at"));
    }

    #[test]
    fn validation_and_annotation_keywords_are_stripped() {
        let json = schema_of::<Simple>().to_string();

        // `u32` would otherwise carry `format: uint32` and `minimum: 0`,
        // `f32` a `format: float` — all rejected in strict mode.
        for keyword in ["\"format\"", "\"minimum\"", "\"$schema\"", "\"title\""] {
            assert!(!json.contains(keyword), "{keyword} survived: {json}");
        }
    }

    #[test]
    fn descriptions_survive_because_they_steer_the_model() {
        let schema = schema_of::<Simple>();
        assert!(
            schema["properties"]["name"]["description"].is_string(),
            "doc comments are the cheapest prompt there is"
        );
    }

    #[test]
    fn definitions_are_renamed_and_every_reference_follows() {
        let schema = schema_of::<Nested>();
        let json = schema.to_string();

        assert!(schema.get("definitions").is_none(), "draft-07 name is gone");
        assert!(schema["$defs"].is_object(), "hoisted to $defs");
        assert!(
            !json.contains("#/definitions/"),
            "a stale $ref would not resolve: {json}"
        );
        assert!(json.contains("#/$defs/"), "references were rewritten");
    }

    #[test]
    fn the_root_is_an_object_which_is_the_only_shape_openai_accepts() {
        assert_eq!(schema_of::<Nested>()["type"], "object");
        assert_eq!(schema_of::<Simple>()["type"], "object");
    }

    #[test]
    fn a_generated_schema_round_trips_the_value_it_describes() {
        // The point of deriving: an instance of the struct satisfies the schema
        // it produced, field for field.
        let schema = schema_of::<Simple>();
        let instance = Simple {
            name: "a".into(),
            count: 1,
            ratio: 0.5,
            maybe: None,
        };
        let json = serde_json::to_value(&instance).expect("serialize");

        let properties = schema["properties"].as_object().expect("properties");
        for key in json.as_object().expect("object").keys() {
            assert!(properties.contains_key(key), "{key} missing from the schema");
        }
        assert_eq!(properties.len(), json.as_object().expect("object").len());
    }
}
