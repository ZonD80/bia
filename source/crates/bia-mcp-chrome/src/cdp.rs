//! Minimal CDP client: evaluate one expression in a tab over Chrome's remote-debugging
//! WebSocket (no extension involved). Used where X cannot see Chrome's own UI, e.g. the
//! folder-upload confirmation drawn inside the browser window.
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use crate::{proc, watch};

/// WebSocket URLs of the tabs titled `title`; every tab when only one is open.
pub fn tabs_titled(title: &str) -> Vec<String> {
    let Ok(list) = watch::json_list() else { return vec![] };
    let pages: Vec<&Value> = list
        .as_array()
        .map(|a| a.iter().filter(|t| t["type"] == "page").collect())
        .unwrap_or_default();
    let ws = |t: &&Value| t["webSocketDebuggerUrl"].as_str().map(str::to_string);
    let named: Vec<String> = pages.iter().filter(|t| t["title"] == title).filter_map(ws).collect();
    if !named.is_empty() || pages.len() != 1 {
        return named;
    }
    pages.iter().filter_map(ws).collect()
}

/// `Runtime.evaluate(expr)` in the tab at `ws_url`; its value.
pub fn eval(ws_url: &str, expr: &str) -> Result<Value, String> {
    let path = ws_url
        .split_once("://")
        .and_then(|(_, rest)| rest.split_once('/'))
        .map(|(_, p)| format!("/{p}"))
        .ok_or_else(|| format!("bad websocket url {ws_url}"))?;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], proc::DEBUG_PORT));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(500)).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_secs(3))).map_err(|e| e.to_string())?;
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n",
        proc::DEBUG_PORT
    )
    .map_err(|e| e.to_string())?;
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        s.read_exact(&mut b).map_err(|e| format!("handshake: {e}"))?;
        head.push(b[0]);
    }
    if !head.starts_with(b"HTTP/1.1 101") {
        return Err(format!("handshake: {}", String::from_utf8_lossy(&head).lines().next().unwrap_or("")));
    }
    let msg = json!({"id": 1, "method": "Runtime.evaluate",
        "params": {"expression": expr, "returnByValue": true}})
    .to_string();
    s.write_all(&frame(msg.as_bytes())).map_err(|e| e.to_string())?;
    // events may come first; the reply carries id 1
    for _ in 0..64 {
        let text = read_frame(&mut s)?;
        let Ok(v) = serde_json::from_slice::<Value>(&text) else { continue };
        if v["id"] == 1 {
            if let Some(e) = v.get("error") {
                return Err(e.to_string());
            }
            return Ok(v["result"]["result"]["value"].clone());
        }
    }
    Err("no reply".into())
}

/// A masked client text frame (the mask may be fixed; it is not a secret).
fn frame(payload: &[u8]) -> Vec<u8> {
    const MASK: [u8; 4] = [0x5a, 0x17, 0xc3, 0x8e];
    let mut f = vec![0x81];
    match payload.len() {
        n if n < 126 => f.push(0x80 | n as u8),
        n if n <= 0xffff => {
            f.push(0x80 | 126);
            f.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            f.push(0x80 | 127);
            f.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    f.extend_from_slice(&MASK);
    f.extend(payload.iter().enumerate().map(|(i, b)| b ^ MASK[i % 4]));
    f
}

/// One server frame's payload (unmasked, unfragmented replies).
fn read_frame(s: &mut TcpStream) -> Result<Vec<u8>, String> {
    let mut h = [0u8; 2];
    s.read_exact(&mut h).map_err(|e| e.to_string())?;
    let len = match h[1] & 0x7f {
        126 => {
            let mut l = [0u8; 2];
            s.read_exact(&mut l).map_err(|e| e.to_string())?;
            u16::from_be_bytes(l) as usize
        }
        127 => {
            let mut l = [0u8; 8];
            s.read_exact(&mut l).map_err(|e| e.to_string())?;
            u64::from_be_bytes(l) as usize
        }
        n => n as usize,
    };
    if len > 64 << 20 {
        return Err("frame too large".into());
    }
    let mut p = vec![0u8; len];
    s.read_exact(&mut p).map_err(|e| e.to_string())?;
    Ok(p)
}

#[cfg(test)]
mod tests {
    #[test]
    fn frames_are_masked_with_length() {
        let f = super::frame(b"abc");
        assert_eq!(&f[..2], &[0x81, 0x83]);
        assert_eq!(f.len(), 2 + 4 + 3);
        let long = super::frame(&[0u8; 300]);
        assert_eq!(&long[..4], &[0x81, 0x80 | 126, 0x01, 0x2c]);
    }
}
