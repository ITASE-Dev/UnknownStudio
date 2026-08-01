//! Tool schemas the model may call, and the parser that turns a returned call
//! into an [`ActionCommand`] the dispatcher can execute.
//!
//! One list drives both provider dialects and the parser, so a tool can never
//! be advertised without being executable.

use crate::ai_tooling::orchestration::dispatcher::ActionCommand;
use serde_json::{json, Map, Value};
use thiserror::Error;

/// A parameter in a tool schema.
struct Param {
    name: &'static str,
    kind: &'static str,
    description: &'static str,
}

const fn param(name: &'static str, kind: &'static str, description: &'static str) -> Param {
    Param {
        name,
        kind,
        description,
    }
}

struct ToolSchema {
    name: &'static str,
    description: &'static str,
    params: &'static [Param],
}

/// Every tool the studio can actually run. Adding one here without a matching
/// arm in [`parse_tool_call`] is a compile-time-invisible bug, so the tests
/// assert the two stay in step.
static TOOLS: &[ToolSchema] = &[
    ToolSchema {
        name: "add_marker",
        description: "Place a labelled marker on the timeline at a given second.",
        params: &[
            param("time_sec", "number", "Timeline position in seconds."),
            param("color", "string", "Marker colour: a name (red, blue, green, amber, purple) or #rrggbb."),
            param("label", "string", "Short label for the marker."),
        ],
    },
    ToolSchema {
        name: "split_clip",
        description: "Cut a clip in two at a timeline second. The time must fall inside the clip.",
        params: &[
            param("clip_id", "string", "Clip id as shown in STATE, e.g. c12."),
            param("time_sec", "number", "Where to cut, in timeline seconds."),
        ],
    },
    ToolSchema {
        name: "delete_clip",
        description: "Remove a clip from the timeline, closing the gap it leaves.",
        params: &[param("clip_id", "string", "Clip id as shown in STATE, e.g. c12.")],
    },
    ToolSchema {
        name: "trim_clip",
        description: "Keep only part of a clip, in that clip's own source seconds.",
        params: &[
            param("clip_id", "string", "Clip id as shown in STATE, e.g. c12."),
            param("start_sec", "number", "New in-point, in source seconds."),
            param("end_sec", "number", "New out-point, in source seconds."),
        ],
    },
];

impl ToolSchema {
    /// JSON Schema for the arguments. Strict mode requires every property to be
    /// listed in `required` and extra ones to be forbidden.
    fn parameters(&self) -> Value {
        let mut properties = Map::new();
        for param in self.params {
            properties.insert(
                param.name.to_string(),
                json!({ "type": param.kind, "description": param.description }),
            );
        }

        json!({
            "type": "object",
            "properties": Value::Object(properties),
            "required": self.params.iter().map(|p| p.name).collect::<Vec<_>>(),
            "additionalProperties": false,
        })
    }
}

/// OpenAI-compatible `tools` array.
pub fn get_available_tools() -> Value {
    Value::Array(
        TOOLS
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "strict": true,
                        "parameters": tool.parameters(),
                    }
                })
            })
            .collect(),
    )
}

/// Anthropic `tools` array — same schemas, different envelope.
pub fn anthropic_tools() -> Value {
    Value::Array(
        TOOLS
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.parameters(),
                })
            })
            .collect(),
    )
}

pub fn tool_names() -> Vec<&'static str> {
    TOOLS.iter().map(|tool| tool.name).collect()
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum ToolCallError {
    #[error("unknown tool: {0}")]
    UnknownTool(String),

    #[error("{tool}: argument '{argument}' is missing or not a {expected}")]
    BadArgument {
        tool: String,
        argument: &'static str,
        expected: &'static str,
    },

    #[error("{tool}: arguments are not a JSON object")]
    MalformedArguments { tool: String },
}

/// Maps one returned tool call to an executable command.
///
/// `arguments` is the object the model produced. Providers hand it over as a
/// JSON *string* (OpenAI) or an object (Anthropic); both are accepted.
pub fn parse_tool_call(name: &str, arguments: &Value) -> Result<ActionCommand, ToolCallError> {
    let object = match arguments {
        Value::Object(map) => map.clone(),
        Value::String(raw) => serde_json::from_str::<Map<String, Value>>(raw)
            .map_err(|_| ToolCallError::MalformedArguments { tool: name.into() })?,
        Value::Null => Map::new(),
        _ => {
            return Err(ToolCallError::MalformedArguments { tool: name.into() });
        }
    };

    match name {
        "add_marker" => Ok(ActionCommand::AddMarker {
            time_sec: number(name, &object, "time_sec")?,
            color: text(name, &object, "color").unwrap_or_else(|_| "blue".into()),
            label: text(name, &object, "label").unwrap_or_default(),
        }),
        "split_clip" => Ok(ActionCommand::SplitClip {
            clip_id: text(name, &object, "clip_id")?,
            time_sec: number(name, &object, "time_sec")?,
        }),
        "delete_clip" => Ok(ActionCommand::DeleteClip {
            clip_id: text(name, &object, "clip_id")?,
        }),
        "trim_clip" => Ok(ActionCommand::TrimClip {
            clip_id: text(name, &object, "clip_id")?,
            start_sec: number(name, &object, "start_sec")?,
            end_sec: number(name, &object, "end_sec")?,
        }),
        other => Err(ToolCallError::UnknownTool(other.to_string())),
    }
}

fn number(tool: &str, object: &Map<String, Value>, argument: &'static str) -> Result<f32, ToolCallError> {
    object
        .get(argument)
        .and_then(|value| match value {
            // Models sometimes quote numbers; accept that rather than refuse.
            Value::String(text) => text.trim().parse::<f64>().ok(),
            other => other.as_f64(),
        })
        .map(|value| value as f32)
        .ok_or(ToolCallError::BadArgument {
            tool: tool.to_string(),
            argument,
            expected: "number",
        })
}

fn text(tool: &str, object: &Map<String, Value>, argument: &'static str) -> Result<String, ToolCallError> {
    object
        .get(argument)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or(ToolCallError::BadArgument {
            tool: tool.to_string(),
            argument,
            expected: "string",
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_openai_schema_is_strict_and_complete() {
        let tools = get_available_tools();
        let array = tools.as_array().expect("array");
        assert_eq!(array.len(), TOOLS.len());

        for tool in array {
            let function = &tool["function"];
            assert_eq!(tool["type"], "function");
            assert_eq!(function["strict"], true);

            let parameters = &function["parameters"];
            assert_eq!(parameters["additionalProperties"], false);

            // Strict mode: every property must also appear in `required`.
            let properties = parameters["properties"].as_object().expect("properties");
            let required = parameters["required"].as_array().expect("required");
            assert_eq!(properties.len(), required.len());
            for key in properties.keys() {
                assert!(required.iter().any(|name| name == key), "{key} not required");
            }
        }
    }

    #[test]
    fn the_four_documented_tools_are_present_with_their_arguments() {
        let tools = get_available_tools();
        let by_name: Vec<&str> = tools
            .as_array()
            .expect("array")
            .iter()
            .map(|tool| tool["function"]["name"].as_str().expect("name"))
            .collect();

        assert_eq!(
            by_name,
            vec!["add_marker", "split_clip", "delete_clip", "trim_clip"]
        );

        let marker = &tools[0]["function"]["parameters"]["properties"];
        assert_eq!(marker["time_sec"]["type"], "number");
        assert_eq!(marker["color"]["type"], "string");
        assert_eq!(marker["label"]["type"], "string");
    }

    #[test]
    fn anthropic_carries_the_same_schemas_in_its_own_envelope() {
        let tools = anthropic_tools();
        let first = &tools[0];
        assert_eq!(first["name"], "add_marker");
        assert_eq!(first["input_schema"]["type"], "object");
        assert!(first.get("function").is_none());
    }

    #[test]
    fn every_advertised_tool_parses_into_a_command() {
        // Nothing may be offered to the model that cannot then be executed.
        for name in tool_names() {
            let arguments = json!({
                "time_sec": 1.0,
                "start_sec": 0.0,
                "end_sec": 2.0,
                "clip_id": "c1",
                "color": "red",
                "label": "x",
            });
            assert!(
                parse_tool_call(name, &arguments).is_ok(),
                "{name} has no parser arm"
            );
        }
    }

    #[test]
    fn arguments_arrive_as_a_string_or_an_object() {
        let as_object = parse_tool_call(
            "split_clip",
            &json!({ "clip_id": "c12", "time_sec": 4.5 }),
        );
        let as_string = parse_tool_call(
            "split_clip",
            &json!(r#"{"clip_id":"c12","time_sec":4.5}"#),
        );

        let expected = ActionCommand::SplitClip {
            clip_id: "c12".into(),
            time_sec: 4.5,
        };
        assert_eq!(as_object, Ok(expected.clone()));
        assert_eq!(as_string, Ok(expected));
    }

    #[test]
    fn quoted_numbers_are_accepted_rather_than_refused() {
        let parsed = parse_tool_call("add_marker", &json!({
            "time_sec": "12.5",
            "color": "red",
            "label": "hook",
        }));

        assert_eq!(
            parsed,
            Ok(ActionCommand::AddMarker {
                time_sec: 12.5,
                color: "red".into(),
                label: "hook".into(),
            })
        );
    }

    #[test]
    fn a_marker_without_styling_still_lands() {
        // Colour and label are cosmetic; a missing one must not lose the marker.
        let parsed = parse_tool_call("add_marker", &json!({ "time_sec": 3.0 }));
        assert_eq!(
            parsed,
            Ok(ActionCommand::AddMarker {
                time_sec: 3.0,
                color: "blue".into(),
                label: String::new(),
            })
        );
    }

    #[test]
    fn missing_essentials_and_unknown_tools_are_reported() {
        let missing = parse_tool_call("split_clip", &json!({ "clip_id": "c1" }));
        assert_eq!(
            missing,
            Err(ToolCallError::BadArgument {
                tool: "split_clip".into(),
                argument: "time_sec",
                expected: "number",
            })
        );

        assert_eq!(
            parse_tool_call("launch_rocket", &json!({})),
            Err(ToolCallError::UnknownTool("launch_rocket".into()))
        );

        assert_eq!(
            parse_tool_call("delete_clip", &json!("not json")),
            Err(ToolCallError::MalformedArguments {
                tool: "delete_clip".into()
            })
        );
    }
}
