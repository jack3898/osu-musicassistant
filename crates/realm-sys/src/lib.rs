//! Raw FFI bindings to realm-core's C API (`realm.h`).
//!
//! Generated at build time by bindgen from the vendored realm-core submodule, which is pinned to
//! the core version shipped with the Realm .NET release osu!lazer uses.

#![allow(non_upper_case_globals, non_camel_case_types, non_snake_case, clippy::all)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
