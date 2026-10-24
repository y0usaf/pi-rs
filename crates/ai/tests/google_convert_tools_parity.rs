//! `google-shared.test.ts`: converting tools to Gemini function declarations.

use pi_ai::api::google::declarations;
use pi_ai::types::Tool;
use serde_json::{Value, json};

fn tool(parameters: Value) -> Tool {
    Tool {
        name: "test_tool".to_owned(),
        description: "A test tool".to_owned(),
        parameters,
        constrained_sampling: None,
    }
}

fn declaration(parameters: Value) -> Value {
    let declarations = declarations(&[tool(parameters)]);
    let wrapped = json!([{"functionDeclarations": declarations}]);
    wrapped[0]["functionDeclarations"][0]["parametersJsonSchema"].clone()
}

#[test]
fn rewrites_const_as_enum() {
    let parameters = json!({
        "type": "object",
        "properties": {
            "origin": {"type": "string", "const": "zq-src-7"},
            "mode": {"anyOf": [
                {"type": "string", "const": "a"},
                {"type": "string", "const": "b"},
            ]},
            "level": {"oneOf": [{"const": 1}, {"const": 2}]},
            "flag": {"type": "boolean", "const": true},
            "list": {"type": "array", "items": {"const": "x"}},
            "const": {"type": "string"},
        },
        "required": ["origin"],
    });
    assert_eq!(
        declaration(parameters),
        json!({
            "type": "object",
            "properties": {
                "origin": {"type": "string", "enum": ["zq-src-7"]},
                "mode": {"anyOf": [
                    {"type": "string", "enum": ["a"]},
                    {"type": "string", "enum": ["b"]},
                ]},
                "level": {"oneOf": [{"enum": [1]}, {"enum": [2]}]},
                "flag": {"type": "boolean", "const": true},
                "list": {"type": "array", "items": {"enum": ["x"]}},
                "const": {"type": "string"},
            },
            "required": ["origin"],
        })
    );
}
