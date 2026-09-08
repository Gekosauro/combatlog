use std::{fs::File, io::{Read, Seek, SeekFrom}, path::Path};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Checkpoint {
    pub offset: u64,
    pub fingerprint: String,
    pub boundary: String,
}

fn fingerprint(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes { hash = (hash ^ *byte as u64).wrapping_mul(0x100000001b3); }
    format!("{hash:016x}:{}", bytes.len())
}

fn range(file: &mut File, start: u64, end: u64) -> Result<Vec<u8>> {
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.take(end - start).read_to_end(&mut bytes)?;
    if bytes.len() as u64 != end - start { bail!("Log changed while reading; retry the upload."); }
    Ok(bytes)
}

fn identity(file: &mut File, len: u64) -> Result<String> {
    let bytes = range(file, 0, len.min(4096))?;
    let end = bytes.iter().position(|b| *b == b'\n').map_or(bytes.len(), |i| i + 1);
    Ok(fingerprint(&bytes[..end]))
}

fn boundary(file: &mut File, offset: u64) -> Result<String> {
    Ok(fingerprint(&range(file, offset.saturating_sub(4096), offset)?))
}

fn complete_end(file: &mut File, len: u64) -> Result<u64> {
    let mut end = len;
    while end > 0 {
        let start = end.saturating_sub(65536);
        let bytes = range(file, start, end)?;
        if let Some(i) = bytes.iter().rposition(|b| *b == b'\n') { return Ok(start + i as u64 + 1); }
        end = start;
    }
    Ok(0)
}

pub fn snapshot(path: &Path) -> Result<Checkpoint> {
    let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let len = file.metadata()?.len();
    let offset = complete_end(&mut file, len)?;
    Ok(Checkpoint { offset, fingerprint: identity(&mut file, len)?, boundary: boundary(&mut file, offset)? })
}

/// A read-only snapshot; the caller persists the checkpoint only after WCL
/// successfully terminates the report. Appends beyond this snapshot are deferred.
pub fn read(path: &Path, checkpoint: Option<&Checkpoint>) -> Result<(String, Checkpoint, bool)> {
    let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let len = file.metadata()?.len();
    let id = identity(&mut file, len)?;
    let mut start = 0;
    let mut reset = false;
    if let Some(cp) = checkpoint {
        if cp.offset == 0 { start = 0; }
        else if cp.offset <= len && cp.fingerprint == id
            && range(&mut file, cp.offset - 1, cp.offset)? == b"\n"
            && boundary(&mut file, cp.offset)? == cp.boundary { start = cp.offset; }
        else { reset = true; }
    }
    let end = complete_end(&mut file, len)?;
    if end <= start { bail!("No complete new log entries since the last successful upload."); }
    let bytes = range(&mut file, start, end)?;
    let next = Checkpoint { offset: end, fingerprint: id, boundary: boundary(&mut file, end)? };
    Ok((String::from_utf8(bytes).context("combat log is not valid UTF-8")?, next, reset))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Log(std::path::PathBuf);
    impl Log {
        fn new() -> Self { Self(std::env::temp_dir().join(format!("combatlog-test-{}-{}.txt", std::process::id(), rand::random::<u64>()))) }
        fn write(&self, bytes: &[u8]) { std::fs::write(&self.0, bytes).unwrap(); }
    }
    impl Drop for Log { fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); } }
    #[test]
    fn append_partial_retry_and_no_new_data() {
        let log = Log::new(); log.write(b"header\nold\npart");
        let (raw, cp, reset) = read(&log.0, None).unwrap();
        assert_eq!(raw, "header\nold\n"); assert!(!reset);
        log.write(b"header\nold\npartial\nnew\n");
        let (raw, next, _) = read(&log.0, Some(&cp)).unwrap();
        assert_eq!(raw, "partial\nnew\n");
        // Failed upload: reuse the unchanged checkpoint, obtaining identical data.
        assert_eq!(read(&log.0, Some(&cp)).unwrap().0, raw);
        assert!(read(&log.0, Some(&next)).is_err());
    }
    #[test]
    fn replacement_and_truncation_reset() {
        let log = Log::new(); log.write(b"header\nold\n");
        let cp = snapshot(&log.0).unwrap();
        log.write(b"header\nNEW\nmore\n");
        let (raw, _, reset) = read(&log.0, Some(&cp)).unwrap();
        assert!(reset); assert_eq!(raw, "header\nNEW\nmore\n");
        log.write(b"x\n"); assert!(read(&log.0, Some(&cp)).unwrap().2);
    }
    #[test]
    fn start_now_is_read_only_and_handles_long_partial_line() {
        let log = Log::new(); let mut bytes = b"header\n".to_vec(); bytes.extend(vec![b'x'; 70000]); log.write(&bytes);
        assert_eq!(snapshot(&log.0).unwrap().offset, 7);
        assert_eq!(std::fs::read(&log.0).unwrap(), bytes);
    }
    #[test]
    fn utf8_crlf_and_empty_file() {
        let log = Log::new(); log.write(b""); assert_eq!(snapshot(&log.0).unwrap().offset, 0);
        assert!(read(&log.0, None).is_err());
        log.write("é\r\n".as_bytes()); assert_eq!(read(&log.0, None).unwrap().1.offset, 4);
    }
}
