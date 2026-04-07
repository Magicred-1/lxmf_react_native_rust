//! lxmf_react_native_rust — React Native bridge for LXMF-rs mesh networking
//!
//! Architecture:
//!   TypeScript (useLxmf hook) → Expo Modules (Swift/Kotlin) → C FFI / JNI → this crate → rns-embedded-ffi
//!
//! This crate wraps the rns-embedded-ffi V1 managed API and adds:
//!   - Beacon announce/discovery (anon0mesh protocol)
//!   - Solana transaction relay via GROUP destinations
//!   - BLE/LoRa frame codecs (HDLC, KISS)
//!   - Message persistence via SQLite

pub mod node;
pub mod beacon;
pub mod ffi;
pub mod framing;
pub mod store;
pub mod reticulum_bridge;

#[cfg(target_os = "android")]
pub mod jni_bridge;
