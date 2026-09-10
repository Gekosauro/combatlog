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
    pub kind: String,
    pub difficulty: u32,
    pub started: String,
    pub ended: Option<String>,
    pub duration_ms: Option<u64>,
    pub complete: bool,
    pub missing_start: bool,
    #[serde(skip)] start: u64,
    #[serde(skip)] end: u64,
    #[serde(skip)] zone: String,
    #[serde(skip)] context: String,
    #[serde(skip)] attempts: u32,
    #[serde(skip)] encounter_open: bool,
}

fn hash(mut value: u64, bytes: &[u8]) -> u64 {
    for b in bytes { value = (value ^ *b as u64).wrapping_mul(0x100000001b3); }
    value
}

fn run_id(run: &Run, digest: u64) -> String {
    format!("{}-v1-{digest:016x}", if run.kind == "raid" { "raid" } else { "mplus" })
}

fn finish(runs: &mut Vec<Run>, active: &mut Option<Run>, digest: u64, closed: bool) {
    if let Some(mut run) = active.take() {
        // A zone entry alone (or a reset END) is not evidence of a finished key.
        if run.missing_start && !run.complete { return; }
        if run.kind == "raid" {
            if run.attempts == 0 { return; }
            run.complete = closed && !run.encounter_open;
        }
        run.id = run_id(&run, digest);
        runs.push(run);
    }
}

/// Stream a fixed file-length snapshot. Recover a missing START only from a
/// Mythic+ zone entry followed by a successful END for that same zone.
pub fn scan(path: &Path) -> Result<Vec<Run>> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let len = file.metadata()?.len();
    let mut reader = BufReader::new(file.take(len));
    let start_re = Regex::new(r#"^CHALLENGE_MODE_START,"([^"]+)",(\d+),(\d+),(\d+),"#)?;
    let zone_re = Regex::new(r#"^ZONE_CHANGE,(\d+),"([^"]+)",(\d+)"#)?;
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
            if let Some(c) = zone_re.captures(event) {
                let difficulty: u32 = c[3].parse()?;
                if active.as_ref().is_some_and(|r| r.kind == "mplus"
                    && (r.zone != c[1] || difficulty != 8)) {
                    finish(&mut runs, &mut active, digest, false);
                }
                // Modern Normal/Heroic/Mythic/LFR and legacy raid difficulties.
                let raid = matches!(difficulty, 3 | 4 | 5 | 6 | 7 | 9 | 14 | 15 | 16 | 17);
                let same_session = active.as_ref().is_some_and(|r|
                    r.kind == "raid" && r.zone == c[1] && r.difficulty == difficulty);
                if active.as_ref().is_some_and(|r| r.kind == "raid") && !same_session {
                    finish(&mut runs, &mut active, digest, true);
                }
                if raid && !same_session {
                    finish(&mut runs, &mut active, digest, false);
                    digest = 0xcbf29ce484222325;
                    active = Some(Run { id: String::new(), name: c[2].into(), level: 0,
                        kind: "raid".into(), difficulty, zone: c[1].into(), started: stamp.into(),
                        ended: None, duration_ms: None, complete: false, missing_start: false, start: offset, end,
                        context: header.clone(), attempts: 0, encounter_open: false });
                }
                if difficulty == 8 && active.is_none() {
                    digest = 0xcbf29ce484222325;
                    active = Some(Run { id: String::new(), name: c[2].into(), level: 0,
                        kind: "mplus".into(), difficulty, zone: c[1].into(), started: stamp.into(),
                        ended: None, duration_ms: None, complete: false, missing_start: true,
                        start: offset, end, context: header.clone(), attempts: 0, encounter_open: false });
                }
            }
            if event.starts_with("COMBAT_LOG_VERSION,") {
                header = line.clone();
                zone.clear();
                map.clear();
            }
            if event.starts_with("ZONE_CHANGE,") { zone = line.clone(); }
            if event.starts_with("MAP_CHANGE,") { map = line.clone(); }
            if let Some(c) = start_re.captures(event) {
                finish(&mut runs, &mut active, digest, true);
                digest = 0xcbf29ce484222325;
                active = Some(Run { id: String::new(), name: c[1].into(), level: c[4].parse()?,
                    kind: "mplus".into(), difficulty: 8, attempts: 0, encounter_open: false,
                    zone: c[2].into(), started: stamp.into(), ended: None, duration_ms: None,
                    complete: false, missing_start: false, start: offset, end,
                    context: format!("{header}{zone}{map}") });
            }
            if let Some(run) = active.as_mut() {
                digest = hash(digest, line.trim_end_matches(['\r', '\n']).as_bytes());
                digest = hash(digest, b"\n");
                run.end = end;
                if run.kind == "raid" {
                    run.ended = Some(stamp.into());
                    if event.starts_with("ENCOUNTER_START,") { run.attempts += 1; run.encounter_open = true; }
                    if event.starts_with("ENCOUNTER_END,") { run.encounter_open = false; }
                }
                if run.kind == "mplus" && event.starts_with("CHALLENGE_MODE_END,") {
                    let fields: Vec<_> = event.split(',').collect();
                    if fields.get(1) == Some(&run.zone.as_str()) {
                        run.complete = fields.get(2) == Some(&"1");
                        run.ended = Some(stamp.into());
                        run.duration_ms = fields.get(4).and_then(|v| v.parse().ok());
                        if run.missing_start {
                            run.level = fields.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
                            run.complete &= run.level > 0 && run.duration_ms.is_some_and(|ms| ms > 0);
                        }
                        finish(&mut runs, &mut active, digest, false);
                    }
                }
            }
        }
        offset = end;
    }
    finish(&mut runs, &mut active, digest, false);
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
    if run_id(&run, digest) != id { bail!("Run changed while reading. Scan again."); }
    let label = if run.kind == "raid" { format!("{} (raid, difficulty {})", run.name, run.difficulty) }
        else { format!("{} +{}", run.name, run.level) };
    Ok((format!("{}{raw}", run.context), format!("{label} — {}", run.started)))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(events: &[&str]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("missing-start-{}.txt", rand::random::<u64>()));
        let data = events.iter().enumerate()
            .map(|(i, event)| format!("9/10/2026 10:00:{i:02}.0000  {event}\n")).collect::<String>();
        std::fs::write(&path, data).unwrap();
        path
    }

    #[test]
    fn finished_key_without_start_is_recovered_and_extracted_separately() {
        // Minimal reproduction of the September 10 log; no player/combat data.
        let path = fixture(&[
            "COMBAT_LOG_VERSION,22", "ZONE_CHANGE,2521,\"Ruby Life Pools\",23",
            "MAP_CHANGE,2095,\"Ruby Life Pools\",0,0,0,0",
            "CHALLENGE_MODE_END,2521,0,0,0",
            "CHALLENGE_MODE_START,\"Ruby Life Pools\",2521,399,10,[]",
            "CHALLENGE_MODE_END,2521,1,10,1291167",
            "ZONE_CHANGE,0,\"Silvermoon City\",0", "COMBAT_LOG_VERSION,22",
            "ZONE_CHANGE,1762,\"Kings' Rest\",8", "MAP_CHANGE,1004,\"Kings' Rest\",0,0,0,0",
            "ENCOUNTER_START,2139,\"The Golden Serpent\",8,5,1762",
            // Repeated metadata must not drop the previously recorded part.
            "ZONE_CHANGE,1762,\"Kings' Rest\",8",
            "ENCOUNTER_END,2139,\"The Golden Serpent\",8,5,1,161602",
            "CHALLENGE_MODE_END,1762,1,12,1566288",
        ]);
        let runs = scan(&path).unwrap();
        assert_eq!(runs.len(), 2);
        assert!(runs.iter().all(|r| r.complete));
        assert!(!runs[0].missing_start);
        assert_eq!(runs[1].name, "Kings' Rest");
        assert_eq!(runs[1].level, 12);
        assert_eq!(runs[1].duration_ms, Some(1566288));
        assert!(runs[1].missing_start);
        let raw = extract(&path, &runs[1].id).unwrap().0;
        assert!(raw.starts_with("9/10/2026 10:00:07.0000  COMBAT_LOG_VERSION,22\n"));
        assert!(raw.contains("ENCOUNTER_START,2139"));
        assert!(raw.contains("CHALLENGE_MODE_END,1762,1,12,1566288"));
        assert!(!raw.contains("Ruby"));
        assert!(!raw.contains("Silvermoon"));
        assert!(!raw.contains("CHALLENGE_MODE_START")); // Never invent missing events.
        let original = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, original.replace('\n', "\r\n")).unwrap();
        assert_eq!(scan(&path).unwrap()[1].id, runs[1].id);
        assert!(extract(&path, &runs[1].id).is_ok());
        std::fs::write(&path, original + "9/10/2026 11:10:00.0000  ZONE_CHANGE,0,\"City\",0\n").unwrap();
        assert_eq!(scan(&path).unwrap()[1].id, runs[1].id);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn orphan_end_reset_open_key_and_wrong_zone_do_not_create_runs() {
        for events in [
            vec!["CHALLENGE_MODE_END,1762,1,12,60000"],
            vec!["ZONE_CHANGE,1762,\"Kings' Rest\",23", "CHALLENGE_MODE_END,1762,1,12,60000"],
            vec!["ZONE_CHANGE,1762,\"Kings' Rest\",8"],
            vec!["ZONE_CHANGE,1762,\"Kings' Rest\",8", "CHALLENGE_MODE_END,1762,0,0,0"],
            vec!["ZONE_CHANGE,1762,\"Kings' Rest\",8", "CHALLENGE_MODE_END,2521,1,12,60000"],
            vec!["ZONE_CHANGE,1762,\"Kings' Rest\",8", "ZONE_CHANGE,0,\"City\",0", "CHALLENGE_MODE_END,1762,1,12,60000"],
            vec!["ZONE_CHANGE,1762,\"Kings' Rest\",8", "CHALLENGE_MODE_END,1762,1,0,0"],
        ] {
            let path = fixture(&events);
            assert!(scan(&path).unwrap().is_empty(), "{events:?}");
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn real_start_supersedes_tentative_zone_and_keeps_existing_report_id() {
        let events = ["ZONE_CHANGE,1762,\"Kings' Rest\",8",
            "CHALLENGE_MODE_START,\"Kings' Rest\",1762,249,12,[]",
            "CHALLENGE_MODE_END,1762,1,12,60000"];
        let path = fixture(&events);
        let runs = scan(&path).unwrap();
        assert_eq!(runs.len(), 1);
        assert!(!runs[0].missing_start);
        let original = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, original.lines().skip(1).collect::<Vec<_>>().join("\n") + "\n").unwrap();
        assert_eq!(scan(&path).unwrap()[0].id, runs[0].id);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn raid_wipes_and_kills_stay_together_and_reentry_is_separate() {
        let path = std::env::temp_dir().join(format!("raids-{}.txt", rand::random::<u64>()));
        let events = [
            "COMBAT_LOG_VERSION,22", "ZONE_CHANGE,3004,\"The Venomous Abyss\",14",
            "ENCOUNTER_START,3492,\"Ula'tek\",14,20,3004", "ENCOUNTER_END,3492,\"Ula'tek\",14,20,0,1000",
            // Repeated zone metadata / logging toggle must not split the raid.
            "COMBAT_LOG_VERSION,22", "ZONE_CHANGE,3004,\"The Venomous Abyss\",14",
            "ENCOUNTER_START,3492,\"Ula'tek\",14,20,3004", "ENCOUNTER_END,3492,\"Ula'tek\",14,20,1,1000",
            "ZONE_CHANGE,0,\"Silvermoon City\",0",
            "ZONE_CHANGE,1762,\"Kings' Rest\",23",
            "CHALLENGE_MODE_START,\"Kings' Rest\",1762,249,10,[]", "CHALLENGE_MODE_END,1762,1,10,60000",
            "ZONE_CHANGE,3004,\"The Venomous Abyss\",15",
            "ENCOUNTER_START,3492,\"Ula'tek\",15,20,3004", "ENCOUNTER_END,3492,\"Ula'tek\",15,20,0,1000",
            "ZONE_CHANGE,0,\"Silvermoon City\",0",
        ];
        let data = events.iter().enumerate().map(|(i,e)| format!("9/7/2026 22:00:{i:02}.0000  {e}\n")).collect::<String>();
        std::fs::write(&path, &data).unwrap();
        let runs = scan(&path).unwrap(); assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].kind, "raid"); assert_eq!(runs[0].attempts, 2);
        assert_eq!(runs[1].kind, "mplus"); assert_eq!(runs[2].difficulty, 15);
        assert!(runs.iter().all(|r| r.complete));
        let raw = extract(&path, &runs[0].id).unwrap().0;
        assert_eq!(raw.matches("ENCOUNTER_END").count(), 2);
        assert!(!raw.contains("Silvermoon")); assert!(!raw.contains("CHALLENGE_MODE_START"));
        // A growing file without a zone exit must not be marked uploaded yet.
        std::fs::write(&path, data.lines().take(8).collect::<Vec<_>>().join("\n") + "\n").unwrap();
        assert!(!scan(&path).unwrap()[0].complete);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    #[ignore = "requires a local log fixture; never publish user combat logs"]
    fn supplied_evening_log_contains_one_raid_and_four_mplus() {
        let path = std::env::var("COMBATLOG_TEST_FILE").unwrap();
        let runs = scan(Path::new(&path)).unwrap();
        assert_eq!(runs.len(), 5);
        assert_eq!(runs[0].name, "The Venomous Abyss");
        assert_eq!(runs[0].attempts, 6);
        assert!(runs.iter().all(|r| r.complete));
        assert_eq!(runs.iter().filter(|r| r.kind == "mplus").count(), 4);
        for run in &runs { assert!(extract(Path::new(&path), &run.id).is_ok()); }
    }
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
