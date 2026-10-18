//! Blocking client for the newline-JSON transports — used by CLI tools, the
//! MCP adapter, and out-of-process tests.

use std::io::{BufRead, BufReader, Write};
#[cfg(all(feature = "uds", unix))]
use std::path::Path;

use crate::protocol::{AgentError, Command, Response, WireRequest, WireResponse};

/// A blocking connection to a running app's control socket.
pub struct Client<S: Read + Write> {
    reader: BufReader<ReadHalf<S>>,
    writer: WriteHalf<S>,
    next_id: u64,
}

use std::io::Read;
use std::sync::Arc;

/// Shared-stream halves so one underlying duplex stream serves both
/// buffered reads and writes.
struct ReadHalf<S>(Arc<S>);
struct WriteHalf<S>(Arc<S>);

impl<S> Read for ReadHalf<S>
where
    for<'a> &'a S: Read,
{
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        (&*self.0).read(buf)
    }
}

impl<S> Write for WriteHalf<S>
where
    for<'a> &'a S: Write,
{
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        (&*self.0).write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        (&*self.0).flush()
    }
}

impl<S> Client<S>
where
    S: Read + Write,
    for<'a> &'a S: Read + Write,
{
    /// Wrap an already-connected duplex stream, performing the auth handshake
    /// if `token` is given.
    pub fn from_stream(stream: S, token: Option<&str>) -> std::io::Result<Self> {
        let stream = Arc::new(stream);
        let mut client = Self {
            reader: BufReader::new(ReadHalf(stream.clone())),
            writer: WriteHalf(stream),
            next_id: 1,
        };
        if let Some(token) = token {
            client.send_json(&serde_json::json!({ "auth": token }))?;
            let line = client.read_line()?;
            if !line.contains("authenticated") {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!("auth rejected: {line}"),
                ));
            }
        }
        Ok(client)
    }

    /// Send a command and block for its response.
    pub fn request(&mut self, command: Command) -> std::io::Result<Response> {
        let id = self.next_id;
        self.next_id += 1;
        self.send_json(&WireRequest { id, command })?;
        let line = self.read_line()?;
        let resp: WireResponse = serde_json::from_str(&line).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("bad response line {line:?}: {e}"),
            )
        })?;
        if resp.id != id {
            return Ok(Err(AgentError::Internal(format!(
                "response id {} does not match request id {id}",
                resp.id
            ))));
        }
        Ok(resp.into_response())
    }

    fn send_json(&mut self, value: &impl serde::Serialize) -> std::io::Result<()> {
        let json = serde_json::to_string(value)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        writeln!(self.writer, "{json}")?;
        self.writer.flush()
    }

    fn read_line(&mut self) -> std::io::Result<String> {
        let mut line = String::new();
        let n = self.reader.read_line(&mut line)?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "connection closed",
            ));
        }
        Ok(line.trim_end().to_owned())
    }
}

/// Connect to a UDS control socket.
#[cfg(all(feature = "uds", unix))]
pub fn connect_uds(
    path: impl AsRef<Path>,
    token: Option<&str>,
) -> std::io::Result<Client<std::os::unix::net::UnixStream>> {
    let stream = std::os::unix::net::UnixStream::connect(path)?;
    // Name the stream type: see `connect_pipe`.
    Client::<std::os::unix::net::UnixStream>::from_stream(stream, token)
}

/// Connect to a named local-socket control channel served by
/// [`crate::Transport::Pipe`]: a named pipe on Windows, a namespaced Unix
/// socket elsewhere. The name must match the server's (e.g. `"myapp.agent"`).
#[cfg(feature = "pipe")]
pub fn connect_pipe(
    name: &str,
    token: Option<&str>,
) -> std::io::Result<Client<interprocess::local_socket::Stream>> {
    use interprocess::local_socket::traits::Stream as _;
    use interprocess::local_socket::{GenericNamespaced, Stream, ToNsName};

    let ns_name = name.to_owned().to_ns_name::<GenericNamespaced>()?;
    let stream = Stream::connect(ns_name)?;
    // Name the stream type explicitly. Left to inference, the `for<'a> &'a S: Read + Write`
    // bound is checked before `S` is known; on macOS objc2's blanket
    // `impl Read for &Retained<T>` then sends the solver into infinite recursion (E0275).
    Client::<Stream>::from_stream(stream, token)
}
