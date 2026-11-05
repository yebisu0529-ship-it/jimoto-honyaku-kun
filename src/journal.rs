//! Append-only, local JSONL translation history. No content leaves this machine.
use anyhow::{Context, Result};
use std::{
    collections::{BTreeSet, HashMap},
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

const PREFIX: &str = "plamo-timeline-";
const LOAD_LIMIT: usize = 10_000;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Record {
    pub id: u64,
    pub start_ms: u64,
    pub end_ms: u64,
    pub source: String,
    pub translation: String,
    pub kind: String,
    pub video_id: String,
    pub created_at: String,
}

pub struct Journal {
    pub path: PathBuf,
}

impl Journal {
    /// Open the newest journal created by this application, ignoring unrelated files.
    /// A damaged final event is recovered by rotating to a new file; the old file
    /// remains untouched, so appending cannot make its corrupt tail a middle line.
    pub fn open_latest(dir: &Path) -> Result<(Self, Vec<Record>)> {
        fs::create_dir_all(dir).context("Could not create the translation history directory")?;
        let mut latest: Option<PathBuf> = None;
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_file()
                || !owned_filename(&entry.file_name().to_string_lossy())
            {
                continue;
            }
            let path = entry.path();
            if latest
                .as_ref()
                .is_none_or(|current| path.file_name() > current.file_name())
            {
                latest = Some(path);
            }
        }
        let Some(path) = latest else {
            return Ok((Self::new(dir)?, Vec::new()));
        };
        let (records, recovered_tail) = load(&path)?;
        if recovered_tail {
            let path = fresh_path(dir);
            let temporary = path.with_extension("recovering");
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            for record in &records {
                serde_json::to_writer(&mut file, record)?;
                file.write_all(b"\n")?;
            }
            file.sync_data()?;
            drop(file);
            // Incomplete recovery snapshots are not selected as readable journals.
            fs::rename(&temporary, &path)?;
            return Ok((Self { path }, records));
        }
        Ok((Self { path }, records))
    }

    pub fn new(dir: &Path) -> Result<Self> {
        fs::create_dir_all(dir).context("Could not create the translation history directory")?;
        let path = fresh_path(dir);
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| format!("Could not create journal {}", path.display()))?;
        file.sync_data()?;
        Ok(Self { path })
    }

    /// An additional event with the same ID replaces that record when loaded.
    /// Call from the application's single journal writer to preserve event order.
    pub fn append(&self, record: &Record) -> Result<()> {
        anyhow::ensure!(
            record.end_ms >= record.start_ms,
            "Translation history interval ends before it starts"
        );
        let mut bytes = serde_json::to_vec(record)?;
        bytes.push(b'\n');
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .open(&self.path)
            .with_context(|| {
                format!(
                    "Could not open journal {} for appending",
                    self.path.display()
                )
            })?;
        // A complete final event may lack a newline (for example after manual import).
        // Add a separator without rewriting even one byte of existing history.
        if file.metadata()?.len() > 0 {
            file.seek(SeekFrom::End(-1))?;
            let mut last = [0u8];
            file.read_exact(&mut last)?;
            if last[0] != b'\n' {
                bytes.insert(0, b'\n');
            }
        }
        file.write_all(&bytes)
            .context("Could not append translation history")?;
        file.sync_data()
            .context("Could not flush translation history to disk")?;
        Ok(())
    }
}

fn fresh_path(dir: &Path) -> PathBuf {
    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%9fZ");
    dir.join(format!(
        "{PREFIX}{timestamp}-{}.jsonl",
        uuid::Uuid::new_v4()
    ))
}

fn owned_filename(name: &str) -> bool {
    let Some(stem) = name
        .strip_prefix(PREFIX)
        .and_then(|s| s.strip_suffix(".jsonl"))
    else {
        return false;
    };
    let Some((timestamp, id)) = stem.split_once('-') else {
        return false;
    };
    timestamp.len() == 25
        && chrono::NaiveDateTime::parse_from_str(timestamp, "%Y%m%dT%H%M%S%9fZ").is_ok()
        && uuid::Uuid::parse_str(id).is_ok()
}

fn load(path: &Path) -> Result<(Vec<Record>, bool)> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut records: HashMap<u64, (u64, Record)> = HashMap::new();
    let mut order = BTreeSet::new();
    let mut pending: Option<(u64, Vec<u8>)> = None;
    let mut number = 0u64;
    loop {
        let mut bytes = Vec::new();
        if reader.read_until(b'\n', &mut bytes)? == 0 {
            break;
        }
        number += 1;
        if bytes.iter().all(|b| b.is_ascii_whitespace()) {
            continue;
        }
        if let Some((line, previous)) = pending.take() {
            let record = decode(&previous, line, path)?;
            retain(record, line, &mut records, &mut order);
        }
        pending = Some((number, bytes));
    }
    let mut recovered = false;
    if let Some((line, bytes)) = pending {
        match decode(&bytes, line, path) {
            Ok(record) => retain(record, line, &mut records, &mut order),
            Err(_) => recovered = true,
        }
    }
    let mut result: Vec<Record> = records.into_values().map(|(_, record)| record).collect();
    // IDs represent insertion order. Source clocks can reset on seeks, video
    // changes, or manual translations; they must not reorder the saved timeline.
    result.sort_by_key(|record| record.id);
    Ok((result, recovered))
}

fn decode(bytes: &[u8], line: u64, path: &Path) -> Result<Record> {
    let record: Record = serde_json::from_slice(bytes).with_context(|| {
        format!(
            "Malformed translation history at {} line {line}",
            path.display()
        )
    })?;
    anyhow::ensure!(
        record.end_ms >= record.start_ms,
        "Invalid translation interval at {} line {line}",
        path.display()
    );
    Ok(record)
}

fn retain(
    record: Record,
    sequence: u64,
    records: &mut HashMap<u64, (u64, Record)>,
    order: &mut BTreeSet<(u64, u64)>,
) {
    let id = record.id;
    if let Some((previous, _)) = records.insert(id, (sequence, record)) {
        order.remove(&(previous, id));
    }
    order.insert((sequence, id));
    if records.len() > LOAD_LIMIT
        && let Some((_, oldest)) = order.pop_first()
    {
        records.remove(&oldest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let p =
                std::env::temp_dir().join(format!("plamo-journal-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn record(id: u64) -> Record {
        Record {
            id,
            start_ms: id * 1000,
            end_ms: id * 1000 + 900,
            source: "Hello\nworld".into(),
            translation: "こんにちは\n世界".into(),
            kind: "youtube".into(),
            video_id: "example".into(),
            created_at: "2026-11-05T00:00:00Z".into(),
        }
    }

    #[test]
    fn append_upserts_and_returns_insertion_ids_in_order() {
        let temp = Temp::new();
        let journal = Journal::new(&temp.0).unwrap();
        assert!(owned_filename(
            journal.path.file_name().unwrap().to_str().unwrap()
        ));
        journal.append(&record(2)).unwrap();
        journal.append(&record(1)).unwrap();
        let before = fs::read(&journal.path).unwrap();
        let mut updated = record(1);
        updated.translation = "更新済み".into();
        journal.append(&updated).unwrap();
        assert!(fs::read(&journal.path).unwrap().starts_with(&before));
        let (_, records) = Journal::open_latest(&temp.0).unwrap();
        assert_eq!(records.iter().map(|r| r.id).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(records[0].translation, "更新済み");
    }

    #[test]
    fn source_clock_resets_and_updates_do_not_reorder_the_timeline() {
        let temp = Temp::new();
        let journal = Journal::new(&temp.0).unwrap();
        let mut first = record(1);
        first.start_ms = 40_000;
        first.end_ms = 42_000;
        journal.append(&first).unwrap();
        let mut after_seek = record(2);
        after_seek.start_ms = 1_000;
        after_seek.end_ms = 2_000;
        journal.append(&after_seek).unwrap();
        let mut manual = record(3);
        manual.start_ms = 0;
        manual.end_ms = 0;
        manual.kind = "text".into();
        journal.append(&manual).unwrap();
        after_seek.translation = "字幕更新".into();
        journal.append(&after_seek).unwrap();
        let (_, records) = Journal::open_latest(&temp.0).unwrap();
        assert_eq!(
            records.iter().map(|record| record.id).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(records[1].translation, "字幕更新");
        assert_eq!(records[2].start_ms, 0);
    }

    #[test]
    fn corrupt_tail_is_recovered_without_modifying_original() {
        let temp = Temp::new();
        let journal = Journal::new(&temp.0).unwrap();
        journal.append(&record(1)).unwrap();
        OpenOptions::new()
            .append(true)
            .open(&journal.path)
            .unwrap()
            .write_all(b"{\"id\":2,\"source\":\"truncated")
            .unwrap();
        let original = fs::read(&journal.path).unwrap();
        let (recovered, records) = Journal::open_latest(&temp.0).unwrap();
        assert_eq!(records.len(), 1);
        assert_ne!(recovered.path, journal.path);
        assert_eq!(fs::read(&journal.path).unwrap(), original);
        recovered.append(&record(2)).unwrap();
        assert_eq!(load(&recovered.path).unwrap().0.len(), 2);
    }

    #[test]
    fn middle_corruption_is_an_error_with_its_line() {
        let temp = Temp::new();
        let journal = Journal::new(&temp.0).unwrap();
        let mut file = OpenOptions::new().append(true).open(&journal.path).unwrap();
        writeln!(file, "broken").unwrap();
        writeln!(file, "{}", serde_json::to_string(&record(1)).unwrap()).unwrap();
        assert!(
            load(&journal.path)
                .unwrap_err()
                .to_string()
                .contains("line 1")
        );
        fs::write(temp.0.join("unrelated.jsonl"), "not history").unwrap();
        assert!(!owned_filename("unrelated.jsonl"));
    }

    #[test]
    fn missing_final_newline_is_separated_and_load_is_bounded() {
        let temp = Temp::new();
        let journal = Journal::new(&temp.0).unwrap();
        fs::write(&journal.path, serde_json::to_vec(&record(1)).unwrap()).unwrap();
        journal.append(&record(2)).unwrap();
        assert_eq!(load(&journal.path).unwrap().0.len(), 2);
        let mut file = OpenOptions::new().append(true).open(&journal.path).unwrap();
        for id in 3..=(LOAD_LIMIT as u64 + 4) {
            writeln!(file, "{}", serde_json::to_string(&record(id)).unwrap()).unwrap();
        }
        let count = fs::metadata(&journal.path).unwrap().len();
        let records = load(&journal.path).unwrap().0;
        assert_eq!(records.len(), LOAD_LIMIT);
        assert_eq!(records[0].id, 5);
        assert_eq!(fs::metadata(&journal.path).unwrap().len(), count);
    }
}
