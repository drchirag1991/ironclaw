//! Signal service layer — adapted from presage/libsignal-service-rs.
//!
//! This module contains Signal Protocol service logic vendored from
//! the presage and libsignal-service-rs projects, adapted to use
//! WASM host functions for I/O instead of reqwest/tokio.
//!
//! Source: https://github.com/whisperfish/libsignal-service-rs
//! License: AGPL-3.0 (upstream), vendored under compatible terms.

pub mod provisioning;
