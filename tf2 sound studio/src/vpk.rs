use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const MAGIC: u32 = 0x55AA_1234;
pub const PACK_MISC: u8 = 0;
pub const PACK_VO: u8 = 1;

/// Sanity cap so a corrupted directory cannot make us allocate gigabytes.
const MAX_ENTRY: u32 = 64 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: String,
    pub pack: u8,
    pub arch: u16,
    pub off: u32,
    pub len: u32,
}

impl Entry {
    pub fn key(&self) -> (u8, u16, u32, u32) {
        (self.pack, self.arch, self.off, self.len)
    }
}

fn pack_prefix(pack: u8) -> &'static str {
    match pack {
        PACK_VO => "tf2_sound_vo_english",
        _ => "tf2_sound_misc",
    }
}

pub fn chunk_path(tf_dir: &Path, pack: u8, arch: u16) -> PathBuf {
    tf_dir.join(format!("{}_{:03}.vpk", pack_prefix(pack), arch))
}

pub fn dir_vpk_path(tf_dir: &Path, pack: u8) -> PathBuf {
    tf_dir.join(format!("{}_dir.vpk", pack_prefix(pack)))
}

fn u16at(data: &[u8], p: usize) -> Option<u16> {
    let b = data.get(p..p + 2)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
}
fn u32at(data: &[u8], p: usize) -> Option<u32> {
    let b = data.get(p..p + 4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn cstr(data: &[u8], pos: &mut usize) -> Option<String> {
    let rest = data.get(*pos..)?;
    let len = rest.iter().position(|&b| b == 0)?;
    let s = String::from_utf8_lossy(&rest[..len]).to_string();
    *pos += len + 1;
    Some(s)
}

fn bad(msg: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, msg.to_string())
}

/// Parse a `*_dir.vpk` tree. Corrupt input returns an error instead of panicking.
pub fn index_pack(tf_dir: &Path, pack: u8) -> std::io::Result<Vec<Entry>> {
    let path = dir_vpk_path(tf_dir, pack);
    let mut data = Vec::new();
    File::open(&path)?.read_to_end(&mut data)?;
    if data.len() < 12 || u32at(&data, 0) != Some(MAGIC) {
        return Err(bad("not a VPK directory file"));
    }
    let version = u32at(&data, 4).ok_or_else(|| bad("truncated VPK header"))?;
    let tree_size = u32at(&data, 8).ok_or_else(|| bad("truncated VPK header"))? as usize;
    let (mut pos, tree_end) = if version == 2 { (28usize, 28 + tree_size) } else { (12usize, 12 + tree_size) };
    let tree_end = tree_end.min(data.len());

    let mut out = Vec::new();
    while pos < tree_end {
        let ext = match cstr(&data, &mut pos) {
            Some(e) => e,
            None => break,
        };
        if ext.is_empty() {
            break;
        }
        loop {
            let dir = match cstr(&data, &mut pos) {
                Some(d) => d,
                None => break,
            };
            if dir.is_empty() {
                break;
            }
            loop {
                let name = match cstr(&data, &mut pos) {
                    Some(n) => n,
                    None => break,
                };
                if name.is_empty() {
                    break;
                }
                let preload = u16at(&data, pos + 4).ok_or_else(|| bad("truncated entry"))?;
                let arch = u16at(&data, pos + 6).ok_or_else(|| bad("truncated entry"))?;
                let off = u32at(&data, pos + 8).ok_or_else(|| bad("truncated entry"))?;
                let len = u32at(&data, pos + 12).ok_or_else(|| bad("truncated entry"))?;
                let term = u16at(&data, pos + 16).ok_or_else(|| bad("truncated entry"))?;
                if term != 0xFFFF {
                    return Err(bad("bad entry terminator"));
                }
                pos += 18 + preload as usize;
                if len > MAX_ENTRY {
                    continue;
                }
                out.push(Entry {
                    path: format!("{}/{}.{}", dir, name, ext),
                    pack,
                    arch,
                    off,
                    len,
                });
            }
        }
    }
    Ok(out)
}

pub fn read_entry(tf_dir: &Path, e: &Entry) -> std::io::Result<Vec<u8>> {
    if e.len > MAX_ENTRY {
        return Err(bad("entry too large"));
    }
    let mut buf = vec![0u8; e.len as usize];
    let path = if e.arch == 0xFFFF {
        dir_vpk_path(tf_dir, e.pack)
    } else {
        chunk_path(tf_dir, e.pack, e.arch)
    };
    let mut f = File::open(&path)?;
    let meta_len = f.metadata()?.len();
    if e.off as u64 + e.len as u64 > meta_len {
        return Err(bad("entry outside archive"));
    }
    f.seek(SeekFrom::Start(e.off as u64))?;
    f.read_exact(&mut buf)?;
    Ok(buf)
}

/// Overwrite entry bytes in place. Length must not change.
pub fn write_entry(tf_dir: &Path, e: &Entry, data: &[u8]) -> std::io::Result<()> {
    if data.len() != e.len as usize {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "replacement must keep the original length",
        ));
    }
    let path = if e.arch == 0xFFFF {
        dir_vpk_path(tf_dir, e.pack)
    } else {
        chunk_path(tf_dir, e.pack, e.arch)
    };
    let mut f = std::fs::OpenOptions::new().write(true).open(&path)?;
    let meta_len = f.metadata()?.len();
    if e.off as u64 + e.len as u64 > meta_len {
        return Err(bad("entry outside archive"));
    }
    f.seek(SeekFrom::Start(e.off as u64))?;
    f.write_all(data)?;
    f.flush()?;
    Ok(())
}

pub fn can_write(tf_dir: &Path, pack: u8) -> bool {
    let p = chunk_path(tf_dir, pack, 0);
    std::fs::OpenOptions::new().write(true).open(p).is_ok()
}
