//! Block-aligned I/O over a raw disk.
//!
//! macOS `/dev/rdisk*` and Windows `\\.\PhysicalDrive*` reject reads and writes whose
//! offset or length is not a multiple of the sector size (EINVAL). The gpt crate and
//! fatfs issue small unaligned writes (headers, FAT entries, directory slots), so every
//! access goes through here: aligned bulk transfers pass straight through, everything
//! else is read-modify-write on one cached block that is written back when another
//! block is touched, on `flush`, or on drop.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};

/// Multiple of both 512-byte and 4Kn sector sizes.
const BLOCK: u64 = 4096;

#[derive(Debug)]
pub struct BlockIo {
    inner: File,
    pos: u64,
    /// Index of the cached block, its bytes, and whether it needs writing back.
    cached: Option<(u64, Box<[u8]>, bool)>,
}

impl BlockIo {
    pub fn new(inner: File) -> Self {
        Self {
            inner,
            pos: 0,
            cached: None,
        }
    }

    pub fn sync_all(&mut self) -> io::Result<()> {
        self.write_back()?;
        self.inner.sync_all()
    }

    fn write_back(&mut self) -> io::Result<()> {
        if let Some((idx, buf, dirty)) = self.cached.as_mut() {
            if *dirty {
                self.inner.seek(SeekFrom::Start(*idx * BLOCK))?;
                self.inner.write_all(buf)?;
                *dirty = false;
            }
        }
        Ok(())
    }

    /// Load block `idx` into the cache (writing back whatever was there).
    fn block(&mut self, idx: u64) -> io::Result<&mut [u8]> {
        if self.cached.as_ref().map(|c| c.0) != Some(idx) {
            self.write_back()?;
            let mut buf = vec![0u8; BLOCK as usize].into_boxed_slice();
            self.inner.seek(SeekFrom::Start(idx * BLOCK))?;
            read_full(&mut self.inner, &mut buf)?;
            self.cached = Some((idx, buf, false));
        }
        Ok(&mut self.cached.as_mut().unwrap().1)
    }

    /// Does the cached block overlap `[start, start+len)`?
    fn cache_overlaps(&self, start: u64, len: u64) -> bool {
        self.cached
            .as_ref()
            .is_some_and(|c| c.0 * BLOCK < start + len && start < (c.0 + 1) * BLOCK)
    }
}

/// Fill `buf` from the device; past the end reads as zeros (image files may be short).
fn read_full(f: &mut File, buf: &mut [u8]) -> io::Result<()> {
    let mut got = 0;
    while got < buf.len() {
        match f.read(&mut buf[got..])? {
            0 => break,
            n => got += n,
        }
    }
    buf[got..].fill(0);
    Ok(())
}

impl Read for BlockIo {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let off = self.pos % BLOCK;
        let bulk = buf.len() as u64 / BLOCK * BLOCK;
        if off == 0 && bulk > 0 && !self.cache_overlaps(self.pos, bulk) {
            self.inner.seek(SeekFrom::Start(self.pos))?;
            let n = self.inner.read(&mut buf[..bulk as usize])?;
            let n = n / BLOCK as usize * BLOCK as usize;
            if n > 0 {
                self.pos += n as u64;
                return Ok(n);
            }
        }
        let idx = self.pos / BLOCK;
        let n = buf.len().min((BLOCK - off) as usize);
        let block = self.block(idx)?;
        buf[..n].copy_from_slice(&block[off as usize..off as usize + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Write for BlockIo {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let off = self.pos % BLOCK;
        let bulk = buf.len() as u64 / BLOCK * BLOCK;
        if off == 0 && bulk > 0 {
            if self.cache_overlaps(self.pos, bulk) {
                self.cached = None; // fully overwritten below
            }
            self.inner.seek(SeekFrom::Start(self.pos))?;
            self.inner.write_all(&buf[..bulk as usize])?;
            self.pos += bulk;
            return Ok(bulk as usize);
        }
        let idx = self.pos / BLOCK;
        let n = buf.len().min((BLOCK - off) as usize);
        let block = self.block(idx)?;
        block[off as usize..off as usize + n].copy_from_slice(&buf[..n]);
        self.cached.as_mut().unwrap().2 = true;
        self.pos += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.write_back()?;
        self.inner.flush()
    }
}

impl Seek for BlockIo {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.pos = match pos {
            SeekFrom::Start(o) => o,
            SeekFrom::Current(o) => self
                .pos
                .checked_add_signed(o)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek before start"))?,
            // Raw devices report 0 here; callers that need the size pass it explicitly.
            SeekFrom::End(_) => {
                self.write_back()?;
                self.inner.seek(pos)?
            }
        };
        Ok(self.pos)
    }
}

impl Drop for BlockIo {
    fn drop(&mut self) {
        let _ = self.write_back();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unaligned_round_trip() {
        let path = std::env::temp_dir().join(format!("bia-blockio-{}", std::process::id()));
        let f = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        f.set_len(5 * BLOCK).unwrap();
        let mut io = BlockIo::new(f);
        let data: Vec<u8> = (0..3 * BLOCK as usize + 77).map(|i| (i % 251) as u8).collect();
        io.seek(SeekFrom::Start(100)).unwrap();
        io.write_all(&data).unwrap();
        io.seek(SeekFrom::Start(BLOCK)).unwrap();
        io.write_all(&data[..2 * BLOCK as usize]).unwrap();
        io.flush().unwrap();

        let mut want = vec![0u8; 5 * BLOCK as usize];
        want[100..100 + data.len()].copy_from_slice(&data);
        want[BLOCK as usize..3 * BLOCK as usize].copy_from_slice(&data[..2 * BLOCK as usize]);
        let mut got = vec![0u8; want.len()];
        io.seek(SeekFrom::Start(0)).unwrap();
        io.read_exact(&mut got).unwrap();
        assert_eq!(got, want);
        assert_eq!(std::fs::read(&path).unwrap(), want);
        drop(io);
        std::fs::remove_file(&path).unwrap();
    }
}
