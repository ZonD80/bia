//! Chrome native-messaging frames: 4-byte little-endian length, then JSON.
//! Chrome accepts at most 1 MiB towards the extension and sends up to 64 MiB from it.
use serde_json::Value;
use std::io::{self, Read, Write};

pub const MAX_TO_CHROME: usize = 1024 * 1024;
pub const MAX_IN: usize = 64 * 1024 * 1024;

/// `InvalidData` means one bad frame that was fully consumed; the stream is still usable.
pub fn read_frame<R: Read>(r: &mut R) -> io::Result<Value> {
    let mut lenb = [0u8; 4];
    r.read_exact(&mut lenb)?;
    let n = u32::from_le_bytes(lenb) as usize;
    if n > MAX_IN {
        io::copy(&mut r.take(n as u64), &mut io::sink())?;
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("frame length {n}")));
    }
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf)?;
    serde_json::from_slice(&buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn write_frame<W: Write>(w: &mut W, v: &Value) -> io::Result<()> {
    write_capped(w, v, usize::MAX)
}

/// Chrome closes the port on a frame over 1 MiB, so refuse it before writing anything.
pub fn write_frame_to_chrome<W: Write>(w: &mut W, v: &Value) -> io::Result<()> {
    write_capped(w, v, MAX_TO_CHROME)
}

fn write_capped<W: Write>(w: &mut W, v: &Value, max: usize) -> io::Result<()> {
    let buf = serde_json::to_vec(v).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if buf.len() > max {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("frame of {} bytes exceeds {max}", buf.len())));
    }
    w.write_all(&(buf.len() as u32).to_le_bytes())?;
    w.write_all(&buf)?;
    w.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bad_frame_keeps_stream() {
        let mut data = Vec::new();
        data.extend_from_slice(&3u32.to_le_bytes());
        data.extend_from_slice(b"{x}");
        write_frame(&mut data, &json!({"ok": 1})).unwrap();
        let mut r = &data[..];
        assert_eq!(read_frame(&mut r).unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(read_frame(&mut r).unwrap(), json!({"ok": 1}));
    }

    #[test]
    fn chrome_cap() {
        let big = json!({"v": "x".repeat(MAX_TO_CHROME)});
        let mut out = Vec::new();
        assert!(write_frame_to_chrome(&mut out, &big).is_err());
        assert!(out.is_empty());
    }
}
