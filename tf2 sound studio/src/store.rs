use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

pub type Key = (u8, u16, u32, u32); // pack, arch, offset, len

const MAX_RECORDS: usize = 200_000;
const MAX_ENTRY: u32 = 64 * 1024 * 1024;

pub fn backup_path(tf_dir: &Path) -> PathBuf {
    tf_dir.join("sound_studio_backup.bin")
}

fn rd_u16(d: &[u8], p: usize) -> Option<u16> {
    let b = d.get(p..p + 2)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
}
fn rd_u32(d: &[u8], p: usize) -> Option<u32> {
    let b = d.get(p..p + 4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn silent_pcm(d: &[u8]) -> bool {
    match d.windows(4).position(|w| w == b"data") {
        Some(i) => d[i + 8..].iter().take(2048).all(|b| *b == 0),
        None => false,
    }
}

/// Records are stored back to back with no header, so a single edit can append one
/// record instead of rewriting the whole backup file.
fn parse_records(data: &[u8]) -> Vec<(Key, Vec<u8>)> {
    let mut out = Vec::new();
    let mut p = 0usize;
    while p + 11 <= data.len() && out.len() < MAX_RECORDS {
        let pack = data[p];
        let arch = match rd_u16(data, p + 1) {
            Some(v) => v,
            None => break,
        };
        let off = match rd_u32(data, p + 3) {
            Some(v) => v,
            None => break,
        };
        let len = match rd_u32(data, p + 7) {
            Some(v) => v,
            None => break,
        };
        if len > MAX_ENTRY {
            break;
        }
        let body = p + 11;
        let end = match body.checked_add(len as usize) {
            Some(e) if e <= data.len() => e,
            _ => break,
        };
        out.push(((pack, arch, off, len), data[body..end].to_vec()));
        p = end;
    }
    out
}

/// Old files started with a record count. Recognise them so existing backups keep working.
fn parse_legacy(data: &[u8]) -> Option<Vec<(Key, Vec<u8>)>> {
    let count = rd_u32(data, 0)? as usize;
    if count == 0 || count > MAX_RECORDS {
        return None;
    }
    let mut p = 4usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let pack = *data.get(p)?;
        let arch = rd_u16(data, p + 1)?;
        let off = rd_u32(data, p + 3)?;
        let len = rd_u32(data, p + 7)?;
        if len > MAX_ENTRY {
            return None;
        }
        let body = p + 11;
        let end = body.checked_add(len as usize)?;
        if end > data.len() {
            return None;
        }
        out.push(((pack, arch, off, len), data[body..end].to_vec()));
        p = end;
    }
    if p != data.len() {
        return None;
    }
    Some(out)
}

/// Tolerant load: damaged data yields whatever records are valid, never a panic.
pub fn load(tf_dir: &Path) -> HashMap<Key, Vec<u8>> {
    let data = match std::fs::read(backup_path(tf_dir)) {
        Ok(d) => d,
        Err(_) => return HashMap::new(),
    };
    if data.is_empty() {
        return HashMap::new();
    }
    if let Some(records) = parse_legacy(&data) {
        let map: HashMap<Key, Vec<u8>> = records.into_iter().collect();
        let _ = save(tf_dir, &map); // migrate to the append-friendly format
        return map;
    }
    parse_records(&data).into_iter().collect()
}

fn write_record(out: &mut Vec<u8>, (pack, arch, off, len): Key, data: &[u8]) {
    out.push(pack);
    out.extend_from_slice(&arch.to_le_bytes());
    out.extend_from_slice(&off.to_le_bytes());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(data);
}

/// Rewrite the whole backup (used for compaction after restores).
pub fn save(tf_dir: &Path, map: &HashMap<Key, Vec<u8>>) -> std::io::Result<()> {
    let mut out = Vec::with_capacity(map.len() * (11 + 4096));
    for (key, data) in map {
        write_record(&mut out, *key, data);
    }
    std::fs::write(backup_path(tf_dir), out)
}

/// Durably add one original wave before the game file is touched. O(1) append.
pub fn append(tf_dir: &Path, key: Key, data: &[u8]) -> std::io::Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(backup_path(tf_dir))?;
    let mut rec = Vec::with_capacity(11 + data.len());
    write_record(&mut rec, key, data);
    f.write_all(&rec)?;
    f.flush()?;
    Ok(())
}

/// Import legacy python-tool backups (arch, off, len, data records, sound misc pack).
pub fn import_legacy(tf_dir: &Path, map: &mut HashMap<Key, Vec<u8>>) -> usize {
    let mut added = 0usize;
    for name in ["cart_silence_backup.bin", "footstep_gain_backup.bin"] {
        let data = match std::fs::read(tf_dir.join(name)) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let count = match rd_u32(&data, 0) {
            Some(c) => (c as usize).min(MAX_RECORDS),
            None => continue,
        };
        let mut q = 4usize;
        for _ in 0..count {
            let arch = match rd_u32(&data, q) {
                Some(v) => v as u16,
                None => break,
            };
            let off = match rd_u32(&data, q + 4) {
                Some(v) => v,
                None => break,
            };
            let len = match rd_u32(&data, q + 8) {
                Some(v) => v,
                None => break,
            };
            if len > MAX_ENTRY {
                break;
            }
            q += 12;
            let end = match q.checked_add(len as usize) {
                Some(e) if e <= data.len() => e,
                _ => break,
            };
            let candidate = data[q..end].to_vec();
            let key = (0u8, arch, off, len);
            match map.get(&key) {
                Some(existing) => {
                    if silent_pcm(existing) && !silent_pcm(&candidate) {
                        map.insert(key, candidate);
                        added += 1;
                    }
                }
                None => {
                    map.insert(key, candidate);
                    added += 1;
                }
            }
            q = end;
        }
    }
    added
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tf2ss_{}_{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn corrupt_backup_does_not_panic() {
        let d = tmp("corrupt");
        std::fs::write(backup_path(&d), vec![0xFFu8; 20]).unwrap();
        let map = load(&d);
        assert!(map.is_empty());
    }

    #[test]
    fn append_and_legacy_migration() {
        let d = tmp("append");
        append(&d, (0, 3, 100, 4), &[1, 2, 3, 4]).unwrap();
        append(&d, (0, 3, 200, 2), &[9, 9]).unwrap();
        let map = load(&d);
        assert_eq!(map.len(), 2);
        assert_eq!(map.get(&(0, 3, 200, 2)), Some(&vec![9, 9]));

        // legacy count-prefixed file still loads and gets rewritten
        let mut legacy = Vec::new();
        legacy.extend_from_slice(&1u32.to_le_bytes());
        write_record(&mut legacy, (0, 3, 300, 1), &[7]);
        std::fs::write(backup_path(&d), legacy).unwrap();
        let map2 = load(&d);
        assert_eq!(map2.get(&(0, 3, 300, 1)), Some(&vec![7]));
    }
}
