//! Chrome spawns this process with no arguments via the native-host wrapper.
//! It only forwards native-messaging frames to the long-lived MCP socket.
use serde_json::json;
use std::io::{self, ErrorKind};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::frame;

pub fn sock_path() -> PathBuf {
    std::env::var_os("BIA_CHROME_SOCK").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/run/bia/chrome.sock"))
}

pub fn run() -> io::Result<()> {
    let sock = UnixStream::connect(sock_path())?;
    sock.set_nonblocking(false)?;
    let mut sock_r = sock.try_clone()?;
    // Both directions can write to the MCP (forwarded frames and oversize errors).
    let sock_w = Arc::new(Mutex::new(sock));
    let sock_err = sock_w.clone();
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let thr = std::thread::spawn(move || {
        let mut stdin = stdin.lock();
        loop {
            match frame::read_frame(&mut stdin) {
                Ok(v) => {
                    if frame::write_frame(&mut *sock_w.lock().unwrap(), &v).is_err() {
                        break;
                    }
                }
                Err(e) if e.kind() == ErrorKind::InvalidData => continue,
                Err(_) => break,
            }
        }
        let _ = sock_w.lock().unwrap().shutdown(std::net::Shutdown::Both);
    });
    loop {
        match frame::read_frame(&mut sock_r) {
            Ok(v) => match frame::write_frame_to_chrome(&mut stdout, &v) {
                Ok(()) => {}
                // Too big for Chrome: answer the caller instead of dropping the port.
                Err(e) if e.kind() == ErrorKind::InvalidData => {
                    if let Some(id) = v.get("id") {
                        let _ = frame::write_frame(&mut *sock_err.lock().unwrap(), &json!({"id": id, "error": format!("native host: {e}")}));
                    }
                }
                Err(_) => break,
            },
            Err(e) if e.kind() == ErrorKind::InvalidData => continue,
            Err(_) => break,
        }
    }
    let _ = thr.join();
    Ok(())
}
