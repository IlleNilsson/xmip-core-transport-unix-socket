#![forbid(unsafe_code)]

//! Streams that arrive over a Unix domain socket. One connection is one
//! Stream.
//!
//! A socket on the file system is how two processes on one Unix box speak
//! without a port: the receiver binds a path, the sender connects to it,
//! and the kernel carries the bytes with no network stack between them.
//! A Receive Location binds its path and takes one connection to its end;
//! a Send Location connects, writes the Stream and closes, which is how the
//! receiver knows the Stream is whole. The peer's credentials are the
//! kernel's word on who connected, an inferred identity in ADR-0019's
//! terms, as a TCP peer address is.
//!
//! Windows has had `AF_UNIX` since 2018 and the Rust standard library does
//! not reach it, so on Windows this transport refuses every send and
//! receive with a permanent error rather than pretend: proven where the
//! operating system has the object, refusing elsewhere. A Receive
//! Location's path is claimed before its socket is bound
//! (`net::socket_file`; ADR-0018, amendment 2026-11-19): a socket a
//! stopped listener left is replaced; another listener holding the path,
//! a socket something answers on, or a file of another kind, which is
//! kept, refuses the Location; and only its own socket file goes with it.
//!
//! The origin URI is the bound path: `unix:///run/xmip/orders.sock`. A
//! send target is a path, or `unix://` and a path.
//!
//! **Acceptance is at-most-once here** ([`AT_MOST_ONCE`]): the connection
//! frames the Stream, closed to end it, and there is no reply on it, so the
//! sender's write completed when the kernel took the bytes. The body is the
//! connection, read to its end as the runtime asks.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use context::property::{UNIX_PEER_GID, UNIX_PEER_PID, UNIX_PEER_UID};
use net::Target;
#[cfg(unix)]
use net::socket_file::SocketFile;
use transport::ArrivalIdentity;
use transport::error::{Result, TransportError};
#[cfg(unix)]
use transport::held::Held;
#[cfg(unix)]
use transport::kept::Kept;
use transport::loopback::{FarEnd, LOOPBACK_TIMEOUT, Loopback};
use transport::{Arrived, Configured, Directions, Transport};
use xcore::settings::{Applies, Settings};

/// Why an arrival on a Unix domain socket cannot be acknowledged after the
/// receive cycle.
pub const AT_MOST_ONCE: &str = "a Unix domain socket connection has no reply: the sender's \
                                write completed when the kernel took the bytes, and closing \
                                the connection ends the Stream";

#[derive(Clone)]
pub struct UnixSocketTransport {
    path: PathBuf,
    timeout: Option<Duration>,
    /// The socket bound as a Receive Location's transport is opened
    /// (`Configured::listening`), and every receive takes from.
    #[cfg(unix)]
    receiving: Kept<SocketFile>,
}

impl UnixSocketTransport {
    /// The socket at `path`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            timeout: None,
            #[cfg(unix)]
            receiving: Kept::new(),
        }
    }

    /// Where the socket is.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The origin every connection to this socket shares.
    #[must_use]
    pub fn origin(&self) -> String {
        origin_of(&self.path)
    }

    /// Bind the socket on its path, claimed first (`net::socket_file`): a
    /// socket a stopped listener left is replaced.
    ///
    /// # Errors
    /// Where another listener holds the path, something answers on a
    /// socket there, or a file of another kind is there, which is kept;
    /// where the path is not permitted or its directory is missing.
    #[cfg(unix)]
    pub fn bind(&self) -> Result<SocketFile> {
        use transport::error::classify;
        SocketFile::bind(&self.path).map_err(|e| classify("binding the socket", &e))
    }

    /// The socket bound, and its path: what a Receive Location keeps.
    #[cfg(unix)]
    fn bound(&self) -> Result<(SocketFile, String)> {
        Ok((self.bind()?, self.path.display().to_string()))
    }

    /// Take one connection from an already-bound socket. Its body is the
    /// connection, read to its end as the runtime asks; acceptance is
    /// at-most-once ([`AT_MOST_ONCE`]).
    ///
    /// # Errors
    /// Where the connection could not be accepted.
    #[cfg(unix)]
    pub fn accept_one(&self, socket: &SocketFile) -> Result<Arrived> {
        use transport::error::classify;
        let (stream, _) =
            transport::socket::accept_within(socket.listener(), self.timeout, "nothing connected")?;
        stream
            .set_read_timeout(self.timeout)
            .map_err(|e| classify("setting the read timeout", &e))?;
        let credentials = peer_credentials(&stream);
        Ok(Arrived::new(
            self.origin(),
            stream,
            transport::Acknowledgement::at_most_once(AT_MOST_ONCE),
        )
        .observing_all(credentials))
    }
}

cfg_select! {
    any(target_os = "linux", target_os = "android") => {
        /// The connecting process's credentials, as the kernel reports them
        /// (`SO_PEERCRED`): its user, group and process ids under
        /// `context::property`'s names. None where the system says none, or
        /// the query failed: the arrival then names no more than its path.
        fn peer_credentials(stream: &std::os::unix::net::UnixStream) -> Vec<(String, String)> {
            use context::property::{UNIX_PEER_GID, UNIX_PEER_PID, UNIX_PEER_UID};
            rustix::net::sockopt::socket_peercred(stream).map_or_else(
                |_| Vec::new(),
                |credentials| {
                    vec![
                        (
                            UNIX_PEER_UID.to_string(),
                            credentials.uid.as_raw().to_string(),
                        ),
                        (
                            UNIX_PEER_GID.to_string(),
                            credentials.gid.as_raw().to_string(),
                        ),
                        (
                            UNIX_PEER_PID.to_string(),
                            credentials.pid.as_raw_nonzero().to_string(),
                        ),
                    ]
                },
            )
        }
    }
    unix => {
        /// No credentials where the kernel keeps no `ucred`.
        fn peer_credentials(_: &std::os::unix::net::UnixStream) -> Vec<(String, String)> {
            Vec::new()
        }
    }
    _ => {}
}

/// The path a target names: `unix://` and a path, or the path itself.
#[must_use]
pub fn target_path(target: &str) -> &Path {
    let named = Target::under(<UnixSocketTransport as Configured>::SCHEMES, target);
    Path::new(named.map_or(target, |named| named.after_scheme()))
}

/// `unix://` and the path as `net::uri` writes it.
#[must_use]
pub fn origin_of(path: &Path) -> String {
    format!("unix://{}", net::uri::path_of(path))
}

/// Why this build cannot speak the protocol at all, where it cannot.
#[must_use]
pub fn unsupported() -> Option<TransportError> {
    if cfg!(unix) {
        None
    } else {
        Some(TransportError::permanent(
            "this operating system has no Unix domain sockets the standard library reaches",
        ))
    }
}

impl Transport for UnixSocketTransport {
    fn name(&self) -> &'static str {
        "unix-socket"
    }

    fn directions(&self) -> Directions {
        Directions::BOTH
    }

    fn arrivals(&self) -> transport::Arrivals {
        transport::Arrivals::Unordered("each connection is a Stream of its own")
    }

    fn waiting(&self) -> transport::Waiting {
        transport::Waiting::Waits("the accept waits for a connection")
    }

    /// Take one connection, from the listener its open bound and
    /// kept, read to its end by the runtime. Acceptance is at-most-once
    /// here: the connection has no reply to defer ([`AT_MOST_ONCE`]).
    fn receive_up_to(&self, _most: usize) -> Result<Vec<Arrived>> {
        cfg_select! {
            unix => {
                let listener = self.receiving.listener()?;
                Ok(vec![self.accept_one(listener)?])
            }
            _ => Err(unsupported().unwrap_or_else(|| TransportError::permanent("unreachable"))),
        }
    }

    fn sending(&self) -> transport::Sending {
        transport::Sending::Streamed("the connection carries the body as it is read")
    }

    /// Connect to the socket the target names, write the bytes, close.
    fn send_from(&self, target: &str, outgoing: transport::Outgoing<'_>) -> Result<()> {
        cfg_select! {
            unix => {
                use std::io::Write;
                use transport::error::classify;
                let mut stream = std::os::unix::net::UnixStream::connect(target_path(target))
                    .map_err(|e| classify("connecting to the socket", &e))?;
                // Written as it is read from the Ledger, never whole here.
                outgoing.write_to(&mut stream)?;
                stream
                    .flush()
                    .map_err(|e| classify("flushing to the socket", &e))
            }
            _ => {
                let _ = (target, outgoing);
                Err(unsupported().unwrap_or_else(|| TransportError::permanent("unreachable")))
            }
        }
    }
}

impl Configured for UnixSocketTransport {
    /// The address is the socket's path: where a Receive Location binds; a
    /// Send Location's socket is each send's target.
    const SETTINGS: &'static Settings = &Settings {
        technology: env!("CARGO_PKG_NAME"),
        settings: &[transport::configured::timeout(
            Applies::Receive,
            None,
            "How long a connection is waited for and one that stops sending is \
             waited on; unbounded when left out.",
        )],
    };

    /// The schemes a Location's address is entered with to choose this technology.
    const SCHEMES: &'static [&'static str] = &["unix"];

    fn timeout_kept(&mut self) -> Option<&mut dyn transport::configured::TimeoutKept> {
        Some(&mut self.timeout)
    }

    fn configured(address: &str, _settings: &xcore::settings::Read) -> Result<Self> {
        Ok(Self::new(address))
    }

    /// The listener every receive takes from, bound at the path a Receive
    /// Location names; refused where the system has no Unix domain sockets
    /// ([`unsupported`]), as its receive would be.
    fn listening(&self) -> Result<Option<&str>> {
        cfg_select! {
            unix => self.receiving.listening(|| self.bound()),
            _ => Err(unsupported().unwrap_or_else(|| TransportError::permanent("unreachable"))),
        }
    }
}

impl UnixSocketTransport {
    /// Both ends on this machine: a socket of this process's own in the
    /// temporary directory, the loopback timeout on the read. Each far end
    /// binds a fresh path, so rounds driven at once from several threads do
    /// not take each other's Stream; the address is the socket's path. On a
    /// system without the object the far end refuses, as the transport does.
    #[must_use]
    pub fn loopback() -> Self {
        Self::new(fresh_path()).timing_out_after(LOOPBACK_TIMEOUT)
    }
}

/// A socket path no other far end of this process has: the socket is the
/// address, so two rounds at once need two sockets.
fn fresh_path() -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(1);
    std::env::temp_dir().join(format!(
        "xmip-loopback-{}-{}.sock",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

impl Loopback for UnixSocketTransport {
    fn arrival_identity(&self) -> ArrivalIdentity {
        if cfg!(any(target_os = "linux", target_os = "android")) {
            ArrivalIdentity::Named(&[UNIX_PEER_UID, UNIX_PEER_GID, UNIX_PEER_PID])
        } else {
            ArrivalIdentity::Unnamed("this system keeps no ucred: a peer is named by its path")
        }
    }

    /// Where the OS has no Unix sockets, neither end can stand.
    fn unavailable(&self) -> Option<String> {
        unsupported().map(|error| error.message)
    }

    /// A bound socket waiting for its one connection; its file goes with
    /// it, taken or not.
    fn far_end(&self) -> Result<Box<dyn FarEnd>> {
        cfg_select! {
            unix => {
                let mut transport = self.clone();
                transport.path = fresh_path();
                let socket = transport.bind()?;
                let address = transport.path().display().to_string();
                Ok(Box::new(Held::new(address, move || {
                    transport.accept_one(&socket)?.taken()
                })))
            }
            _ => Err(unsupported().unwrap_or_else(|| TransportError::permanent("unreachable"))),
        }
    }

    fn send_to(&self, address: &str, payload: &[u8]) -> Result<()> {
        Self::new(address).send(address, payload)
    }

    /// A socket on the file system is connected to by its path, not a TCP
    /// connect: connect and close, and the far end reads an empty Stream.
    fn unblock(&self, address: &str) {
        cfg_select! {
            unix => {
                drop(std::os::unix::net::UnixStream::connect(target_path(address)));
            }
            _ => {
                let _ = address;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use transport::payload::edge_payloads;
    use xcore::settings::Given;

    #[test]
    fn unix_socket_declares_its_settings_and_reads_through_them() {
        assert_eq!(
            UnixSocketTransport::SETTINGS.problems(),
            Vec::<String>::new()
        );
        let given = [("timeout".to_string(), Given::Text("2s".to_string()))];
        let path = "/run/xmip/orders.sock";
        let built = UnixSocketTransport::open(path, Applies::Receive, &given).expect("built");
        assert_eq!(built.path(), Path::new(path));
        assert_eq!(built.timeout, Some(Duration::from_secs(2)));
        let Err(refused) = UnixSocketTransport::open(path, Applies::Send, &given) else {
            panic!("a Send Location reads no timeout");
        };
        assert!(refused.message.contains("timeout"), "{}", refused.message);
    }

    #[cfg(unix)]
    #[test]
    fn every_receive_takes_from_the_listener_the_first_bound() {
        let scratch = scratch("kept");
        let receiver =
            UnixSocketTransport::new(scratch.join("in.sock")).timing_out_after(LOOPBACK_TIMEOUT);
        receiver
            .receiving
            .bound(|| receiver.bound())
            .expect("bound");
        let address = receiver.receiving.address().expect("address");
        transport::kept::held_across_receives(&receiver, address, 5, |at, payload| {
            UnixSocketTransport::new(at).send(at, payload)
        });
    }

    /// A folder of a test's own, for its socket and the socket's lock.
    fn scratch(name: &str) -> file::temporary::Folder {
        file::temporary::Folder::new(&format!("unix-{name}"))
    }

    #[cfg(unix)]
    #[test]
    fn the_loopback_returns_the_edge_payloads_whole() {
        let pair = UnixSocketTransport::loopback();
        for (name, bytes) in edge_payloads() {
            let arrived = pair
                .round(&bytes)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(arrived.bytes, bytes, "{name}");
            assert!(arrived.origin_uri.starts_with("unix:///"), "{name}");
        }
        assert!(pair.ceiling().is_none());
        assert!(pair.refuses(b"\r\n\0").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn each_far_end_is_its_own_socket_and_goes_with_its_stream() {
        let pair = UnixSocketTransport::loopback();
        let first = pair.far_end().expect("the first socket");
        let second = pair.far_end().expect("the second socket");
        assert_ne!(first.address(), second.address());
        let address = PathBuf::from(first.address());
        pair.send_to(first.address(), b"once").expect("sending");
        assert_eq!(first.take_one().expect("taking").bytes, b"once");
        assert!(!address.exists(), "the socket file went with its Stream");
    }

    #[cfg(not(unix))]
    #[test]
    fn a_loopback_on_a_system_without_the_object_refuses() {
        let pair = UnixSocketTransport::loopback();
        let error = pair.round(b"x").expect_err("no sockets here");
        assert!(error.message.contains("Unix domain sockets"), "{error}");
        assert!(pair.ceiling().is_none());
        assert!(pair.refuses(b"x").is_none());
        pair.unblock("nowhere");
    }

    #[test]
    fn the_transport_names_itself_and_goes_both_ways() {
        let socket = UnixSocketTransport::new("/run/xmip/orders.sock");
        assert_eq!(socket.name(), "unix-socket");
        assert_eq!(socket.directions(), Directions::BOTH);
        assert!(
            socket.claims().is_none(),
            "a listening socket holds nothing"
        );
        assert_eq!(socket.origin(), "unix:///run/xmip/orders.sock");
    }

    #[test]
    fn a_target_is_a_path_with_or_without_its_scheme() {
        assert_eq!(
            target_path("unix:///run/xmip/a.sock"),
            Path::new("/run/xmip/a.sock")
        );
        assert_eq!(
            target_path("/run/xmip/a.sock"),
            Path::new("/run/xmip/a.sock")
        );
        assert_eq!(
            origin_of(Path::new("C:\\xmip\\a.sock")),
            "unix:///C:/xmip/a.sock"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_connection_is_one_stream_read_to_its_end() {
        let scratch = scratch("stream");
        let path = scratch.join("in.sock");
        let far_end = UnixSocketTransport::new(&path).timing_out_after(Duration::from_secs(2));
        let listener = far_end.bind().expect("binding");
        let target = format!("unix://{}", path.display());
        let sender = std::thread::spawn(move || {
            UnixSocketTransport::new("/nowhere").send(&target, b"over a socket\0\xff")
        });
        let arrived = far_end.accept_one(&listener).expect("accepting");
        assert!(!arrived.defers(), "a socket connection is at-most-once");
        let arrived = arrived.taken().expect("read to its end");
        sender.join().expect("thread").expect("sending");
        assert_eq!(arrived.bytes, b"over a socket\0\xff");
        assert_eq!(arrived.origin_uri, far_end.origin());
    }

    #[cfg(unix)]
    #[test]
    fn a_location_over_an_ordinary_file_is_refused_at_open_and_the_file_kept() {
        let scratch = scratch("ordinary");
        let path = scratch.join("in.sock");
        std::fs::write(&path, b"kept").expect("an ordinary file");
        let location =
            UnixSocketTransport::open(&path.display().to_string(), Applies::Receive, &[])
                .expect("built");
        let refused = location.listening().expect_err("refused");
        assert!(!refused.retryable, "{refused}");
        assert!(refused.message.contains("an ordinary file"), "{refused}");
        assert_eq!(std::fs::read(&path).expect("still there"), b"kept");
    }

    #[cfg(unix)]
    #[test]
    fn a_socket_nobody_listens_on_is_refused() {
        let scratch = scratch("nobody");
        let path = scratch.join("in.sock");
        let error = UnixSocketTransport::new(&path)
            .send(&path.display().to_string(), b"x")
            .expect_err("nothing there");
        assert!(!error.retryable, "{error}");
        assert!(unsupported().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn an_accept_nobody_answers_gives_up_within_its_timeout() {
        // The defect this asserts hung the Essentials' gate: a far end whose
        // near end never came waited for good (2026-09-21).
        let scratch = scratch("unanswered");
        let path = scratch.join("in.sock");
        let transport =
            UnixSocketTransport::new(&path).timing_out_after(Duration::from_millis(200));
        let listener = transport.bind().expect("the socket is bound");
        let began = std::time::Instant::now();

        let refused = transport.accept_one(&listener);
        let waited = began.elapsed();

        assert!(refused.is_err(), "nothing connected, so nothing arrived");
        assert!(
            waited < Duration::from_secs(2),
            "gave up after {waited:?}, which is not a timeout"
        );
        assert!(
            waited >= Duration::from_millis(150),
            "gave up after {waited:?}, before it had waited"
        );
    }

    #[cfg(not(unix))]
    #[test]
    fn a_system_without_the_object_refuses_rather_than_pretends() {
        let scratch = scratch("refused");
        let path = scratch.join("in.sock");
        let socket = UnixSocketTransport::new(&path);
        let receive = socket.receive().expect_err("no sockets here");
        assert!(!receive.retryable, "{receive}");
        let send = socket
            .send(&path.display().to_string(), b"x")
            .expect_err("no sockets here");
        assert!(send.message.contains("Unix domain sockets"), "{send}");
        assert!(unsupported().is_some());
        assert!(!path.exists(), "nothing was created");
    }
}
