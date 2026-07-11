//! Protocol-free helpers — ports of `packages/ai/src/utils` files that the
//! protocol layer shares (spec: `ref/pi` @ `c5582102`, pi v0.79.0).

pub mod headers;
pub mod json_parse;
pub mod sanitize;

pub use headers::headers_to_record;
pub use json_parse::{
    parse_json_with_repair, parse_partial_json, parse_streaming_json, repair_json,
};
pub use sanitize::sanitize_surrogates;
