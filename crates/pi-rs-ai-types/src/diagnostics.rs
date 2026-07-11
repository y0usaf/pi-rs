//! Port of `packages/ai/src/utils/diagnostics.ts` (spec: pi v0.79.0).
//!
//! The spec's `formatThrownValue` / `extractDiagnosticError` operate on JS
//! thrown values (Error name/stack/code); the Rust analogues take a message
//! (or anything `Display`) — `stack` has no Rust equivalent and stays `None`
//! unless a caller sets it.

use serde::{Deserialize, Serialize};

use crate::types::now_ms;

/// Spec: `DiagnosticErrorInfo["code"] = string | number`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DiagnosticCode {
    String(String),
    Number(serde_json::Number),
}

/// Spec: `DiagnosticErrorInfo`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticErrorInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<DiagnosticCode>,
}

impl DiagnosticErrorInfo {
    /// Spec: `extractDiagnosticError` for a non-`Error` thrown value —
    /// `{ name: "ThrownValue", message: String(value) }`.
    pub fn thrown(value: impl std::fmt::Display) -> Self {
        Self {
            name: Some("ThrownValue".to_owned()),
            message: value.to_string(),
            stack: None,
            code: None,
        }
    }
}

/// Spec: `AssistantMessageDiagnostic`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AssistantMessageDiagnostic {
    pub r#type: String,
    /// Unix timestamp in milliseconds.
    pub timestamp: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<DiagnosticErrorInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Map<String, serde_json::Value>>,
}

impl AssistantMessageDiagnostic {
    /// Spec: `createAssistantMessageDiagnostic`.
    pub fn new(
        r#type: impl Into<String>,
        error: DiagnosticErrorInfo,
        details: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Self {
        Self {
            r#type: r#type.into(),
            timestamp: now_ms(),
            error: Some(error),
            details,
        }
    }
}
