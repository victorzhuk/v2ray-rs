//! `v2ray-rs-antidpi` — NFQUEUE-based DPI-circumvention helper.
//!
//! Strategies and their semantics are documented in
//! `docs/antidpi-registry.md`. The library exposes the packet surgery and
//! strategy machinery; the `v2ray-rs-antidpi` binary is the CLI around it.

pub mod engine;
pub mod error;
pub mod flow;
pub mod fooling;
pub mod fw;
mod nft;
pub mod packet;
pub mod positions;
pub mod quic;
pub mod rawsend;
pub mod state;
pub mod strategy;
pub mod tls;
