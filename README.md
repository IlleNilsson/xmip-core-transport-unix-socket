# xmip-core-transport-unix-socket

Unix domain socket transport: one connection to a socket on the file system is one Stream, on the operating systems that have one. A technology of [xmip-core-transport](https://github.com/IlleNilsson/xmip-core-transport).

A Receive Location's accept is bounded through the capability's one bounded accept (`transport::socket::accept_within`), which waits on the socket's readiness and takes a connection as it lands; until 2026-09-27 this crate carried its own copy, polling with a two-millisecond nap.

A Receive Location keeps its listener, bound as its transport is opened (`Configured::listening`; on its first receive until 2026-11-16) and kept (`transport::kept::Kept`): a peer that connects between two receives is queued and taken by the next, where until 2026-09-27 each receive bound the socket anew and a peer between receives was refused. On a system without Unix domain sockets a Receive Location is refused as the node starts, at phase 9, as its receive would be.

A Receive Location's socket and each loopback far end's are bound through `net::socket_file` in [xmip-core-library-net](https://github.com/IlleNilsson/xmip-core-library-net), the one claim of a path a listener binds (ADR-0018, amendment 2026-11-19): `<path>.lock` held while it listens, and removed with it; a socket a stopped listener left is replaced; another listener holding the path, a socket something answers on, or a file of another kind there, which is kept, refuses the Location at phase 9 naming the path; and only the listener's own socket file goes with it. Until 2026-11-19 this crate removed whatever was at its path before it bound — an ordinary file, or the socket of a listener still answering, which went on running unreachable — and removed its path as it went, whatever had taken its place.

A send target is read by `net::Target` in [xmip-core-library-net](https://github.com/IlleNilsson/xmip-core-library-net), the one reading of a URI every technology calls. Until 2026-09-28 this technology stripped its scheme by hand.

## Acknowledgement

Acceptance is at-most-once here. The connection frames the Stream, closed to
end it, and there is no reply on it: the sender's write completed when the
kernel took the bytes, so nobody is told how the receive cycle ended. The body
is the connection, read to its end as the runtime asks, never whole in memory.

A send writes the Message to its connection as it is read from the Ledger, a
piece at a time, never whole in memory (`Sending::Streamed`, 2026-10-19).

## Toolchain

`rust-toolchain.toml` pins the toolchain for the whole estate. Do not change it
here.

## Verification

The included workflow is manual-only and calls the versioned shared workflow at
`IlleNilsson/.github@v1`.
