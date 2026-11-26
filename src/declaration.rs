//! What a Location of this technology is configured with: its declaration,
//! which `Configured` reads, built with the crate's `declaration` feature
//! alone and the code behind the default (ADR-0064, amendments 2026-10-22
//! and 2026-11-26), for the library the surfaces load to carry (open
//! problem 48). It is the same on every operating system, those without
//! Unix domain sockets the standard library reaches included: a Location's
//! settings are suggested and checked wherever it is designed.

use xcore::settings::{Applies, Settings};

/// Every setting a Location takes beyond its address, the socket's path:
/// where a Receive Location binds; a Send Location's socket is each send's
/// target.
pub const SETTINGS: &Settings = &Settings {
    technology: env!("CARGO_PKG_NAME"),
    settings: &[transport::configured::timeout(
        Applies::Receive,
        None,
        "How long a connection is waited for and one that stops sending is \
         waited on; unbounded when left out.",
    )],
};

/// The schemes a Location's address is entered with to choose this technology.
pub const SCHEMES: &[&str] = &["unix"];
