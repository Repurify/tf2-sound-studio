use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const MAGIC: u32 = 0x55AA_1234;

pub const PACK_MISC: u8 = 0;
pub const PACK_VO: u8 = 1;

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

fn cstr(data: &[u8], pos: &mut usize) -> Option<String> {
    let start = *pos;
    let end = *pos + data[start..].iter().position(|&b| b == 0)?;
    *pos = end + 1;
    Some(String::from_utf8_lossy(&data[start..end]).to_string())
}

fn u16at(data: &[u8], p: usize) -> u16 {
    u16::from_le_bytes([data[p], data[p + 1]])
}
fn u32at(data: &[u8], p: usize) -> u32 {
    u32::from_le_bytes([data[p], data[p + 1], data[p + 2], data[p + 3]])
}

/// Parse a `*_dir.vpk` tree into entries. Only VPK 1/2 supported (TF2 uses 2).
pub fn index_pack(tf_dir: &Path, pack: u8) -> std::io::Result<Vec<Entry>> {
    let path = dir_vpk_path(tf_dir, pack);
    let mut data = Vec::new();
    File::open(&path)?.read_to_end(&mut data)?;
    if data.len() < 12 || u32at(&data, 0) != MAGIC {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("not a VPK: {}", path.display()),
        ));
    }
    let version = u32at(&data, 4);
    let (mut pos, tree_end) = if version == 2 {
        (28usize, 28 + u32at(&data, 8) as usize)
    } else {
        (12usize, 12 + u32at(&data, 8) as usize)
    };
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
                if pos + 18 > data.len() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "truncated VPK tree",
                    ));
                }
                let preload = u16at(&data, pos + 4) as usize;
                let arch = u16at(&data, pos + 6);
                let off = u32at(&data, pos + 8);
                let len = u32at(&data, pos + 12);
                pos += 16 + 2 + preload;
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
    let mut buf = vec![0u8; e.len as usize];
    if e.arch == 0xFFFF {
        let mut f = File::open(dir_vpk_path(tf_dir, e.pack))?;
        f.seek(SeekFrom::Start(e.off as u64))?;
        f.read_exact(&mut buf)?;
        return Ok(buf);
    }
    let mut f = File::open(chunk_path(tf_dir, e.pack, e.arch))?;
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
    let mut f = std::fs::OpenOptions::new().write(true).open(path)?;
    f.seek(SeekFrom::Start(e.off as u64))?;
    f.write_all(data)?;
    f.flush()?;
    Ok(())
}

pub fn can_write(tf_dir: &Path, pack: u8) -> bool {
    let p = chunk_path(tf_dir, pack, 0);
    std::fs::OpenOptions::new().write(true).open(p).is_ok()
}
