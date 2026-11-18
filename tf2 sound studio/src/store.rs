use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub type Key = (u8, u16, u32, u32); // pack, arch, offset, len

pub fn backup_path(tf_dir: &Path) -> PathBuf {
    tf_dir.join("sound_studio_backup.bin")
}

fn rd_u16(d: &[u8], p: usize) -> u16 {
    u16::from_le_bytes([d[p], d[p + 1]])
}
fn rd_u32(d: &[u8], p: usize) -> u32 {
    u32::from_le_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]])
}

pub fn load(tf_dir: &Path) -> HashMap<Key, Vec<u8>> {
    let path = backup_path(tf_dir);
    let mut map = HashMap::new();
    let data = match std::fs::read(&path) {
        Ok(d) => d,
        Err(_) => return map,
    };
    if data.len() < 4 {
        return map;
    }
    let count = rd_u32(&data, 0) as usize;
    let mut p = 4usize;
    for _ in 0..count {
        if p + 11 > data.len() {
            break;
        }
        let pack = data[p];
        let arch = rd_u16(&data, p + 1);
        let off = rd_u32(&data, p + 3);
        let len = rd_u32(&data, p + 7);
        p += 11;
        let end = p + len as usize;
        if end > data.len() {
            break;
        }
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
pub fn import_legacy(tf_dir: &Path, map: &mut HashMap<Key, Vec<u8>>) -> usize {
    let mut added = 0usize;
    for name in ["cart_silence_backup.bin", "footstep_gain_backup.bin"] {
        let p = tf_dir.join(name);
        let data = match std::fs::read(&p) {
            Ok(d) => d,
            Err(_) => continue,
        };
        if data.len() < 4 {
            continue;
        }
        let count = rd_u32(&data, 0) as usize;
        let mut q = 4usize;
        for _ in 0..count {
            if q + 12 > data.len() {
                break;
            }
            let arch = rd_u32(&data, q) as u16;
            let off = rd_u32(&data, q + 4);
            let len = rd_u32(&data, q + 8);
            q += 12;
            let end = q + len as usize;
            if end > data.len() {
                break;
            }
            let key = (0u8, arch, off, len);
            let candidate = data[q..end].to_vec();
            let silent = candidate
                .windows(4)
                .position(|w| w == b"data")
                .map(|i| candidate[i + 8..].iter().take(2048).all(|b| *b == 0))
                .unwrap_or(false);
            match map.get(&key) {
                Some(existing) => {
                    let existing_silent = existing
                        .windows(4)
                        .position(|w| w == b"data")
                        .map(|i| existing[i + 8..].iter().take(2048).all(|b| *b == 0))
                        .unwrap_or(false);
                    // legacy backups hold pre-edit audio; replace a silent record with it
                    if existing_silent && !silent {
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
