use std::collections::HashMap;
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

/// Tolerant load: a damaged file yields whatever records are valid, never a panic.
pub fn load(tf_dir: &Path) -> HashMap<Key, Vec<u8>> {
    let mut map = HashMap::new();
    let data = match std::fs::read(backup_path(tf_dir)) {
        Ok(d) => d,
        Err(_) => return map,
    };
    let count = match rd_u32(&data, 0) {
        Some(c) => (c as usize).min(MAX_RECORDS),
        None => return map,
    };
    let mut p = 4usize;
    for _ in 0..count {
        let pack = match data.get(p) {
            Some(v) => *v,
            None => break,
        };
        let arch = match rd_u16(&data, p + 1) {
            Some(v) => v,
            None => break,
        };
        let off = match rd_u32(&data, p + 3) {
            Some(v) => v,
            None => break,
        };
        let len = match rd_u32(&data, p + 7) {
            Some(v) => v,
            None => break,
        };
        if len > MAX_ENTRY {
            break;
        }
        p += 11;
        let end = match p.checked_add(len as usize) {
            Some(e) if e <= data.len() => e,
            _ => break,
        };
        map.insert((pack, arch, off, len), data[p..end].to_vec());
        p = end;
    }
    map
}

pub fn save(tf_dir: &Path, map: &HashMap<Key, Vec<u8>>) -> std::io::Result<()> {
    let mut out = Vec::with_capacity(4 + map.len() * 12);
    out.extend_from_slice(&(map.len() as u32).to_le_bytes());
    for ((pack, arch, off, len), data) in map {
        out.push(*pack);
        out.extend_from_slice(&arch.to_le_bytes());
        out.extend_from_slice(&off.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(data);
    }
    std::fs::write(backup_path(tf_dir), out)
}

/// Import legacy python-tool backups (arch, off, len, data records, sound misc pack).
/// Only fills in entries that are missing or currently silent, so it cannot clobber
/// a good backup with a bad one.
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
    fn roundtrip() {
        let d = tmp("round");
        let mut map = HashMap::new();
        map.insert((0u8, 3u16, 100u32, 4u32), vec![1, 2, 3, 4]);
        save(&d, &map).unwrap();
        let back = load(&d);
        assert_eq!(back.get(&(0u8, 3u16, 100u32, 4u32)), Some(&vec![1, 2, 3, 4]));
    }
}
