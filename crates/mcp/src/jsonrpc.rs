//! `protocol/jsonrpc.ts`: the JSON-RPC envelopes the transports carry, and their classification.

use serde_json::{Map, Value};

use crate::error::{Error, Result};

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;

/// A request id: a number or a string, as the spec allows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Id {
    Number(u64),
    String(String),
}

impl Id {
    pub fn to_value(&self) -> Value {
        match self {
            Self::Number(number) => Value::from(*number),
            Self::String(text) => Value::from(text.clone()),
        }
    }

    fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::Number(number) => number.as_u64().map(Self::Number),
            Value::String(text) => Some(Self::String(text.clone())),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Request {
        id: Id,
        method: String,
        params: Option<Value>,
    },
    Notification {
        method: String,
        params: Option<Value>,
    },
    Response {
        id: Id,
        result: Value,
    },
    ErrorResponse {
        id: Id,
        code: i64,
        message: String,
        data: Option<Value>,
    },
}

pub fn is_object(value: &Value) -> bool {
    value.is_object()
}

fn is_jsonrpc(value: &Map<String, Value>) -> bool {
    value.get("jsonrpc").and_then(Value::as_str) == Some("2.0")
}

fn params(value: &Map<String, Value>) -> Option<Value> {
    value.get("params").cloned()
}

pub fn is_request(value: &Value) -> bool {
    value.as_object().is_some_and(|object| {
        is_jsonrpc(object)
            && object.get("id").and_then(Id::from_value).is_some()
            && object.get("method").and_then(Value::as_str).is_some()
    })
}

pub fn is_notification(value: &Value) -> bool {
    value.as_object().is_some_and(|object| {
        is_jsonrpc(object)
            && !object.contains_key("id")
            && object.get("method").and_then(Value::as_str).is_some()
    })
}

pub fn is_response(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    if !is_jsonrpc(object) || object.get("id").and_then(Id::from_value).is_none() {
        return false;
    }
    if object.contains_key("result") {
        return !object.contains_key("error");
    }
    object
        .get("error")
        .and_then(Value::as_object)
        .is_some_and(|error| {
            error.get("code").and_then(Value::as_i64).is_some()
                && error.get("message").and_then(Value::as_str).is_some()
        })
}

pub fn parse(value: &Value) -> Result<Message> {
    let Some(object) = value.as_object() else {
        return Err(Error::invalid("Invalid JSON-RPC message"));
    };
    if is_request(value) {
        return Ok(Message::Request {
            id: Id::from_value(&object["id"]).expect("a checked id"),
            method: object["method"].as_str().expect("a checked method").into(),
            params: params(object),
        });
    }
    if is_notification(value) {
        return Ok(Message::Notification {
            method: object["method"].as_str().expect("a checked method").into(),
            params: params(object),
        });
    }
    if is_response(value) {
        let id = Id::from_value(&object["id"]).expect("a checked id");
        return match object.get("error") {
            Some(error) => Ok(Message::ErrorResponse {
                id,
                code: error["code"].as_i64().unwrap_or(INTERNAL_ERROR),
                message: error["message"].as_str().unwrap_or_default().into(),
                data: error.get("data").cloned(),
            }),
            None => Ok(Message::Response {
                id,
                result: object.get("result").cloned().unwrap_or(Value::Null),
            }),
        };
    }
    Err(Error::invalid("Invalid JSON-RPC message"))
}

pub fn request(id: &Id, method: &str, params: Option<Value>) -> Value {
    let mut object = Map::new();
    object.insert("jsonrpc".into(), Value::from("2.0"));
    object.insert("id".into(), id.to_value());
    object.insert("method".into(), Value::from(method));
    if let Some(params) = params {
        object.insert("params".into(), params);
    }
    Value::Object(object)
}

pub fn notification(method: &str, params: Option<Value>) -> Value {
    let mut object = Map::new();
    object.insert("jsonrpc".into(), Value::from("2.0"));
    object.insert("method".into(), Value::from(method));
    if let Some(params) = params {
        object.insert("params".into(), params);
    }
    Value::Object(object)
}

pub fn success(id: &Id, result: Value) -> Value {
    let mut object = Map::new();
    object.insert("jsonrpc".into(), Value::from("2.0"));
    object.insert("id".into(), id.to_value());
    object.insert("result".into(), result);
    Value::Object(object)
}

pub fn error_response(id: &Id, code: i64, message: &str) -> Value {
    let mut object = Map::new();
    object.insert("jsonrpc".into(), Value::from("2.0"));
    object.insert("id".into(), id.to_value());
    object.insert(
        "error".into(),
        serde_json::json!({ "code": code, "message": message }),
    );
    Value::Object(object)
}
