//! pi-rs-app — the `packages/coding-agent` port (spec: `ref/pi` @
//! `c5582102`, pi v0.79.0). Thin binary: cli args, config, mode
//! selection; behavior ships as Lua (divergence 2).
//!
//! WS2.6 lands the doctrine-06 bare core: the substrate with zero packs
//! still boots — `pi --login`, `pi --list-models`, `pi "prompt"`
//! streaming a raw completion (no tool loop; the loop is a WS4 pack).
//! Module structure mirrors the spec's `src/` so parity audits stay
//! diff-shaped; modules arrive with their workstreams.

pub mod builtins;
pub mod cli;
pub mod config;
pub mod core;
