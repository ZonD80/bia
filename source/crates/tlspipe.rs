//! A TLS stream (server or client side) shared by a reader and a writer thread. Shared by
//! bia-mcp-web and bia-bridge (`#[path]`).
use rustls::Connection;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Cloneable TLS stream shared by a reader and a writer thread. Blocking socket reads
/// happen outside the lock, so a WebSocket writer is never stuck behind an idle reader.
#[derive(Clone)]
pub struct SharedTls {
    inner: Arc<Mutex<(Connection, TcpStream, Vec<u8>)>>,
    raw: Arc<TcpStream>,
}

impl SharedTls {
    pub fn new(conn: impl Into<Connection>, tcp: TcpStream) -> std::io::Result<Self> {
        let raw = Arc::new(tcp.try_clone()?);
        Ok(SharedTls {
            inner: Arc::new(Mutex::new((conn.into(), tcp, Vec::new()))),
            raw,
        })
    }

    pub fn set_read_timeout(&self, dur: Option<Duration>) -> std::io::Result<()> {
        self.raw.set_read_timeout(dur)
    }

    pub fn set_nodelay(&self, nodelay: bool) -> std::io::Result<()> {
        self.raw.set_nodelay(nodelay)
    }

    pub fn peer_addr(&self) -> std::io::Result<std::net::SocketAddr> {
        self.raw.peer_addr()
    }

    /// The TCP socket underneath (socket options).
    pub fn with_tcp(&self, f: impl FnOnce(&TcpStream)) {
        f(&self.raw)
    }

    pub fn shutdown(&self) {
        let _ = self.raw.shutdown(std::net::Shutdown::Both);
    }
}

fn flush_tls(conn: &mut Connection, tcp: &mut TcpStream) -> std::io::Result<()> {
    while conn.wants_write() {
        conn.write_tls(tcp)?;
    }
    Ok(())
}

impl Read for SharedTls {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut raw = vec![0u8; 16 * 1024];
        loop {
            {
                let mut g = self.inner.lock().unwrap();
                let (conn, tcp, pending) = &mut *g;
                match conn.reader().read(buf) {
                    Ok(n) => return Ok(n),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(e),
                }
                // The plaintext is drained: hand rustls what the socket gave, one read_tls at
                // a time. Feeding several records at once overflows its plaintext buffer
                // ("received plaintext buffer full") when a peer streams large frames.
                if !pending.is_empty() {
                    let mut slice = &pending[..];
                    let k = conn.read_tls(&mut slice)?;
                    if k == 0 {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "tls: stuck",
                        ));
                    }
                    pending.drain(..k);
                    conn.process_new_packets()
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                    flush_tls(conn, tcp)?;
                    continue;
                }
            }
            let n = (&*self.raw).read(&mut raw)?;
            let mut g = self.inner.lock().unwrap();
            let (conn, tcp, pending) = &mut *g;
            if n == 0 {
                // end of stream: rustls decides between a clean close and a truncation
                let mut empty: &[u8] = &[];
                conn.read_tls(&mut empty)?;
                conn.process_new_packets()
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                flush_tls(conn, tcp)?;
                return match conn.reader().read(buf) {
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(0),
                    other => other,
                };
            }
            pending.extend_from_slice(&raw[..n]);
        }
    }
}

impl Write for SharedTls {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut g = self.inner.lock().unwrap();
        let (conn, tcp, _) = &mut *g;
        let n = conn.writer().write(buf)?;
        flush_tls(conn, tcp)?;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let mut g = self.inner.lock().unwrap();
        let (conn, tcp, _) = &mut *g;
        conn.writer().flush()?;
        flush_tls(conn, tcp)
    }
}
