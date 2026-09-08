use std::{fs::File, io::{BufRead, BufReader, Read, Seek, SeekFrom}, path::Path};
use anyhow::{bail, Context, Result};
use regex::Regex;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: String,
    pub name: String,
    pub level: u32,
    pub started: String,
    pub ended: Option<String>,
    pub duration_ms: Option<u64>,
    pub complete: bool,
    #[serde(skip)] start: u64,
    #[serde(skip)] end: u64,
    #[serde(skip)] zone: String,
    #[serde(skip)] context: String,
}

fn hash(mut value: u64, bytes: &[u8]) -> u64 {
    for b in bytes { value = (value ^ *b as u64).wrapping_mul(0x100000001b3); }
    value
}

/// Stream a fixed file-length snapshot. Never infer a run from a reset END.
pub fn scan(path: &Path) -> Result<Vec<Run>> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let len = file.metadata()?.len();
    let mut reader = BufReader::new(file.take(len));
    let start_re = Regex::new(r#"^CHALLENGE_MODE_START,"([^"]+)",(\d+),(\d+),(\d+),"#)?;
    let mut runs = Vec::new();
    let mut active: Option<Run> = None;
    let mut digest = 0xcbf29ce484222325;
    let mut header = String::new();
    let mut zone = String::new();
    let mut map = String::new();
    let mut offset = 0;
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 || !line.ends_with('\n') { break; }
        let end = offset + n as u64;
        if let Some((stamp, event)) = line.trim_end().split_once("  ") {
            if event.starts_with("COMBAT_LOG_VERSION,") { header = line.clone(); }
            if event.starts_with("ZONE_CHANGE,") { zone = line.clone(); }
            if event.starts_with("MAP_CHANGE,") { map = line.clone(); }
            if let Some(c) = start_re.captures(event) {
                if let Some(mut previous) = active.take() {
                    previous.id = format!("mplus-v1-{digest:016x}"); runs.push(previous);
                }
                digest = 0xcbf29ce484222325;
                active = Some(Run { id: String::new(), name: c[1].into(), level: c[4].parse()?,
                    zone: c[2].into(), started: stamp.into(), ended: None, duration_ms: None,
                    complete: false, start: offset, end,
                    context: format!("{header}{zone}{map}") });
            }
            if let Some(run) = active.as_mut() {
                digest = hash(digest, line.trim_end_matches(['\r', '\n']).as_bytes());
                digest = hash(digest, b"\n");
                run.end = end;
                if event.starts_with("CHALLENGE_MODE_END,") {
                    let fields: Vec<_> = event.split(',').collect();
                    if fields.get(1) == Some(&run.zone.as_str()) {
                        run.complete = fields.get(2) == Some(&"1");
                        run.ended = Some(stamp.into());
                        run.duration_ms = fields.get(4).and_then(|v| v.parse().ok());
                        run.id = format!("mplus-v1-{digest:016x}");
                        runs.push(active.take().unwrap());
                    }
                }
            }
        }
        offset = end;
    }
    if let Some(mut run) = active { run.id = format!("mplus-v1-{digest:016x}"); runs.push(run); }
    Ok(runs)
}

/// Re-scan rather than trusting stale offsets supplied by the UI. Only complete
/// runs can be uploaded. Prefix parser metadata, never earlier combat events.
pub fn extract(path: &Path, id: &str) -> Result<(String, String)> {
    let run = scan(path)?.into_iter().find(|r| r.id == id)
        .context("Run no longer matches this file. Scan again.")?;
    if !run.complete { bail!("This run is incomplete or abandoned."); }
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(run.start))?;
    let mut bytes = Vec::new();
    file.take(run.end - run.start).read_to_end(&mut bytes)?;
    if bytes.len() as u64 != run.end - run.start { bail!("Log changed while reading. Scan again."); }
    let raw = String::from_utf8(bytes)?;
    let digest = raw.lines().fold(0xcbf29ce484222325, |h, line| hash(hash(h, line.as_bytes()), b"\n"));
    if format!("mplus-v1-{digest:016x}") != id { bail!("Run changed while reading. Scan again."); }
    Ok((format!("{}{raw}", run.context), format!("{} +{} — {}", run.name, run.level, run.started)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reset_events_are_not_runs_and_reports_are_isolated() {
        let path = std::env::temp_dir().join(format!("runs-{}.txt", rand::random::<u64>()));
        let data = "9/7/2026 22:00:00.0000  COMBAT_LOG_VERSION,22\n9/7/2026 22:00:00.0000  CHALLENGE_MODE_END,1,0,0,0\n9/7/2026 22:01:00.0000  CHALLENGE_MODE_START,\"One\",1,1,10,[]\n9/7/2026 22:02:00.0000  CHALLENGE_MODE_END,1,1,10,60000\n9/7/2026 22:03:00.0000  CHALLENGE_MODE_START,\"Two\",2,2,11,[]\n9/7/2026 22:04:00.0000  CHALLENGE_MODE_END,2,1,11,60000\n";
        std::fs::write(&path, data).unwrap();
        let runs = scan(&path).unwrap(); assert_eq!(runs.len(), 2);
        assert!(runs.iter().all(|r| r.complete)); assert_ne!(runs[0].id, runs[1].id);
        let (raw, _) = extract(&path, &runs[1].id).unwrap();
        assert!(raw.contains("COMBAT_LOG_VERSION")); assert!(!raw.contains("\"One\""));
        std::fs::write(&path, data.replace('\n', "\r\n")).unwrap();
        assert_eq!(scan(&path).unwrap()[0].id, runs[0].id);
        std::fs::write(&path, data.split("9/7/2026 22:04").next().unwrap()).unwrap();
        let incomplete = scan(&path).unwrap(); assert!(!incomplete[1].complete);
        assert!(extract(&path, &incomplete[1].id).is_err());
        let _ = std::fs::remove_file(path);
    }
}
