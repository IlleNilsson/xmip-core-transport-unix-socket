# xmip-core-transport-unix-socket

Unix domain socket transport: one connection to a socket on the file system is one Stream, on the operating systems that have one. A technology of [xmip-core-transport](https://github.com/IlleNilsson/xmip-core-transport).

A Receive Location's accept is bounded through the capability's one bounded accept (`transport::socket::accept_within`), which waits on the socket's readiness and takes a connection as it lands; until 2026-09-27 this crate carried its own copy, polling with a two-millisecond nap.

A Receive Location keeps its listener, bound on the first receive and kept (`transport::kept::Kept`): a peer that connects between two receives is queued and taken by the next, where until 2026-09-27 each receive bound the socket anew and a peer between receives was refused.

A send target is read by `net::Target` in [xmip-core-library-net](https://github.com/IlleNilsson/xmip-core-library-net), the one reading of a URI every technology calls. Until 2026-09-28 this technology stripped its scheme by hand.

## Toolchain

`rust-toolchain.toml` pins the toolchain for the whole estate. Do not change it
here.

## Verification

The included workflow is manual-only and calls the versioned shared workflow at
`IlleNilsson/.github@v1`.
