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

#[test]
fn preserves_literal_data_under_non_schema_keywords() {
    let literal = json!({"const": "literal-data", "properties": {"nested": {"const": 1}}});
    for key in ["default", "examples", "enum", "const", "x-custom"] {
        let value = if key == "enum" || key == "examples" {
            json!([literal])
        } else {
            literal.clone()
        };
        let parameters =
            json!({"type": "object", "properties": {"value": {"type": "object", key: value}}});
        assert_eq!(
            declaration(parameters.clone()),
            parameters,
            "literal data under {key} is preserved"
        );
    }
}

#[test]
fn preserves_names_while_rewriting_schemas_in_name_maps() {
    for key in [
        "properties",
        "patternProperties",
        "$defs",
        "definitions",
        "dependentSchemas",
        "dependencies",
    ] {
        let parameters = json!({
            "type": "object",
            key: {
                "__proto__": {"const": "x"},
                "const": {"const": "y"},
                "properties": {"const": "z"},
            },
        });
        let expected = json!({
            "type": "object",
            key: {
                "__proto__": {"enum": ["x"]},
                "const": {"enum": ["y"]},
                "properties": {"enum": ["z"]},
            },
        });
        let result = declaration(parameters);
        assert_eq!(result, expected, "name map {key}");
        assert_eq!(
            serde_json::to_string(&result).expect("serializes"),
            serde_json::to_string(&expected).expect("serializes"),
            "member order in name map {key}"
        );
    }
}

#[test]
fn rewrites_only_schema_bearing_keywords() {
    let parameters = json!({
        "type": "object",
        "additionalProperties": {"const": "x"},
        "unevaluatedProperties": false,
        "propertyNames": {"const": "name"},
        "allOf": [{"if": {"const": "x"}, "then": {"const": "y"}, "else": {"not": {"const": "z"}}}],
        "properties": {
            "list": {
                "type": "array",
                "items": [{"const": 1}],
                "prefixItems": [{"const": 2}],
                "additionalItems": {"const": 3},
                "contains": {"const": 4},
                "unevaluatedItems": {"const": 5},
            },
        },
        "dependencies": {"list": ["other"]},
    });
    assert_eq!(
        declaration(parameters),
        json!({
            "type": "object",
            "additionalProperties": {"enum": ["x"]},
            "unevaluatedProperties": false,
            "propertyNames": {"enum": ["name"]},
            "allOf": [{"if": {"enum": ["x"]}, "then": {"enum": ["y"]}, "else": {"not": {"enum": ["z"]}}}],
            "properties": {
                "list": {
                    "type": "array",
                    "items": [{"enum": [1]}],
                    "prefixItems": [{"enum": [2]}],
                    "additionalItems": {"enum": [3]},
                    "contains": {"enum": [4]},
                    "unevaluatedItems": {"enum": [5]},
                },
            },
            "dependencies": {"list": ["other"]},
        })
    );
}
