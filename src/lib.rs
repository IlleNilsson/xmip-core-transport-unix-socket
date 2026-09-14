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
//! operating system has the object, refusing elsewhere. The socket file is
//! removed when a Receive Location binds it, because a path the last run
//! left behind is what `bind` fails on.
//!
//! The origin URI is the bound path: `unix:///run/xmip/orders.sock`. A
//! send target is a path, or `unix://` and a path.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use transport::error::{Result, TransportError};
use transport::loopback::{FarEnd, LOOPBACK_TIMEOUT, Loopback};
use transport::{Arrived, Directions, Transport};

/// The listening end, on a system that has one.
#[cfg(unix)]
pub type Listener = std::os::unix::net::UnixListener;

#[derive(Clone)]
pub struct UnixSocketTransport {
    path: PathBuf,
    timeout: Option<Duration>,
}

impl UnixSocketTransport {
    /// The socket at `path`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            timeout: None,
        }
    }

    /// Give up on a connection that stops sending.
    #[must_use]
    pub const fn timing_out_after(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
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

    /// Bind the socket, removing a file a previous run left at the path.
    ///
    /// # Errors
    /// Where the path is not permitted or its directory is missing.
    #[cfg(unix)]
    pub fn bind(&self) -> Result<Listener> {
        use transport::error::classify;
        std::fs::remove_file(&self.path).ok();
        Listener::bind(&self.path).map_err(|e| classify("binding the socket", &e))
    }

    /// Take one connection from an already-bound listener, to its end.
    ///
    /// # Errors
    /// Where the connection could not be accepted or read to its end.
    #[cfg(unix)]
    pub fn accept_one(&self, listener: &Listener) -> Result<Arrived> {
        use std::io::Read;
        use transport::error::classify;
        let (mut stream, _) = listener
            .accept()
            .map_err(|e| classify("accepting a connection", &e))?;
        stream
            .set_read_timeout(self.timeout)
            .map_err(|e| classify("setting the read timeout", &e))?;
        let mut bytes = Vec::new();
        stream
            .read_to_end(&mut bytes)
            .map_err(|e| classify("reading the connection", &e))?;
        Ok(Arrived::new(self.origin(), bytes))
    }
}

/// The path a target names: `unix://` and a path, or the path itself.
#[must_use]
pub fn target_path(target: &str) -> &Path {
    Path::new(target.strip_prefix("unix://").unwrap_or(target))
}

/// `unix://` and the path, forward slashes throughout.
#[must_use]
pub fn origin_of(path: &Path) -> String {
    let text = path.display().to_string().replace(char::from(92), "/");
    let text = text.strip_prefix('/').unwrap_or(&text).to_string();
    format!("unix:///{text}")
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

    /// Bind, and take one connection to its end.
    fn receive(&self) -> Result<Vec<Arrived>> {
        #[cfg(unix)]
        {
            let listener = self.bind()?;
            Ok(vec![self.accept_one(&listener)?])
        }
        #[cfg(not(unix))]
        {
            Err(unsupported().unwrap_or_else(|| TransportError::permanent("unreachable")))
        }
    }

    /// Connect to the socket the target names, write the bytes, close.
    fn send(&self, target: &str, bytes: &[u8]) -> Result<()> {
        #[cfg(unix)]
        {
            use std::io::Write;
            use transport::error::classify;
            let mut stream = std::os::unix::net::UnixStream::connect(target_path(target))
                .map_err(|e| classify("connecting to the socket", &e))?;
            stream
                .write_all(bytes)
                .map_err(|e| classify("writing to the socket", &e))?;
            stream
                .flush()
                .map_err(|e| classify("flushing to the socket", &e))
        }
        #[cfg(not(unix))]
        {
            let _ = (target, bytes);
            Err(unsupported().unwrap_or_else(|| TransportError::permanent("unreachable")))
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

/// A bound socket waiting for its one connection; the file goes with it.
#[cfg(unix)]
struct Bound {
    transport: UnixSocketTransport,
    listener: Listener,
    address: String,
}

#[cfg(unix)]
impl FarEnd for Bound {
    fn address(&self) -> &str {
        &self.address
    }

    fn take_one(self: Box<Self>) -> Result<Arrived> {
        self.transport.accept_one(&self.listener)
    }
}

/// The socket file goes with the far end, taken or not.
#[cfg(unix)]
impl Drop for Bound {
    fn drop(&mut self) {
        std::fs::remove_file(self.transport.path()).ok();
    }
}

impl Loopback for UnixSocketTransport {
    /// Where the OS has no Unix sockets, neither end can stand.
    fn unavailable(&self) -> Option<String> {
        unsupported().map(|error| error.message)
    }

    fn far_end(&self) -> Result<Box<dyn FarEnd>> {
        #[cfg(unix)]
        {
            let mut transport = self.clone();
            transport.path = fresh_path();
            let listener = transport.bind()?;
            let address = transport.path().display().to_string();
            Ok(Box::new(Bound {
                transport,
                listener,
                address,
            }))
        }
        #[cfg(not(unix))]
        {
            Err(unsupported().unwrap_or_else(|| TransportError::permanent("unreachable")))
        }
    }

    fn send_to(&self, address: &str, payload: &[u8]) -> Result<()> {
        Self::new(address).send(address, payload)
    }

    /// A socket on the file system is connected to by its path, not a TCP
    /// connect: connect and close, and the far end reads an empty Stream.
    fn unblock(&self, address: &str) {
        #[cfg(unix)]
        drop(std::os::unix::net::UnixStream::connect(target_path(
            address,
        )));
        #[cfg(not(unix))]
        let _ = address;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use transport::payload::edge_payloads;

    fn scratch(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        std::env::temp_dir().join(format!(
            "xmip-unix-{name}-{}-{nanos}.sock",
            std::process::id()
        ))
    }

    #[cfg(unix)]
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
        let path = scratch("stream");
        let far_end = UnixSocketTransport::new(&path).timing_out_after(Duration::from_secs(2));
        let listener = far_end.bind().expect("binding");
        let target = format!("unix://{}", path.display());
        let sender = std::thread::spawn(move || {
            UnixSocketTransport::new("/nowhere").send(&target, b"over a socket\0\xff")
        });
        let arrived = far_end.accept_one(&listener).expect("accepting");
        sender.join().expect("thread").expect("sending");
        assert_eq!(arrived.bytes, b"over a socket\0\xff");
        assert_eq!(arrived.origin_uri, far_end.origin());
        std::fs::remove_file(&path).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_socket_nobody_listens_on_is_refused() {
        let path = scratch("nobody");
        let error = UnixSocketTransport::new(&path)
            .send(&path.display().to_string(), b"x")
            .expect_err("nothing there");
        assert!(!error.retryable, "{error}");
        assert!(unsupported().is_none());
    }

    #[cfg(not(unix))]
    #[test]
    fn a_system_without_the_object_refuses_rather_than_pretends() {
        let path = scratch("refused");
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
