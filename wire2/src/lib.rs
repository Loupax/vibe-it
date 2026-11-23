//! wire2: local forward proxy whose traffic streams through a chain of external filters (wire2/SPEC.md).
//!
//! Two binaries share this library: `wire2` (the proxy, [`proxy_main`]) and `wire2-pass` (the reference
//! filter `pass`, [`pass::main`]). The integration tests also use [`pass`] to run the test-only
//! pass-decrypt filter (the same forwarding code with `layer: http`). See NOTES.md for the design.

mod app;
mod ca;
mod config;
mod conn;
mod decrypt;
mod filter;
mod flow;
mod host;
mod http;
mod hub;
mod log;
pub mod pass;
mod proxy;
mod splice;
mod upstream;

pub use app::main as proxy_main;
