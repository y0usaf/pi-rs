//! pi-rs-ai — the `packages/ai` port (spec: `ref/pi` @ `c5582102`, pi v0.79.0).
//!
//! Structure follows the locked `pi-rs-ai` decision row, not pi's file
//! sprawl: layered `types → auth → transport → protocols → registry`, no
//! upward deps. Parity is behavioral (recorded fixtures), not module
//! diffs.
//!
//! - types: [`pi_rs_ai_types`] (separate crate, WS2.1)
//! - [`transport`]: HTTP + SSE decode + retry + cancellation, written
//!   exactly once, zero protocol knowledge (WS2.2)
//! - [`protocols`]: wire mapping per API family (WS2.3)
//! - [`util`]: protocol-free helper ports (`json-parse`,
//!   `sanitize-unicode`, `headers`)
//! - [`registry`]: catalog as data + resolution (WS2.4)

pub mod protocols;
pub mod registry;
pub mod transport;
pub mod util;
