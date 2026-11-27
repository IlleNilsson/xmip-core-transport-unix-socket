//! This technology's declaration: the transport guide's "The declaration",
//! `module/core/capability/transport/doc/adding-a-transport.md`. It is the
//! same on every operating system, those without Unix domain sockets the
//! standard library reaches included: a Location's settings are suggested and
//! checked wherever it is designed.

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

/// This technology's schemes, as [`transport::Configured::SCHEMES`] defines
/// them.
pub const SCHEMES: &[&str] = &["unix"];
