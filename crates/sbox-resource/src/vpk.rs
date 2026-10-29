//! Valve package (`.vpk`, version 2) reader. Entries stored in the package itself (archive index
//! `0x7fff`) and in sibling `*_NNN.vpk` archives are supported.

use anyhow::{bail, ensure, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const SIGNATURE: u32 = 0x55aa1234;
const EMBEDDED: u16 = 0x7fff;

#[derive(Clone, Debug)]
pub struct Entry {
    pub preload: Vec<u8>,
    pub archive: u16,
    pub offset: u32,
    pub length: u32,
}

pub struct Vpk {
    path: PathBuf,
    data_start: usize,
    /// Lowercase `dir/name.ext` -> entry.
    pub entries: HashMap<String, Entry>,
    bytes: Vec<u8>,
}

fn cstr(d: &[u8], pos: &mut usize) -> Result<String> {
    let end = d[*pos..].iter().position(|&b| b == 0).ok_or_else(|| anyhow::anyhow!("unterminated string"))?;
    let s = String::from_utf8_lossy(&d[*pos..*pos + end]).into_owned();
    *pos += end + 1;
    Ok(s)
}

impl Vpk {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        Self::parse(path.clone(), std::fs::read(&path)?)
    }

    pub fn parse(path: PathBuf, bytes: Vec<u8>) -> Result<Self> {
        ensure!(bytes.len() >= 28, "vpk too small");
        let u32_at = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
        ensure!(u32_at(0) == SIGNATURE, "not a vpk");
        let version = u32_at(4);
        if version != 2 {
            bail!("unsupported vpk version {version}");
        }
        let tree_size = u32_at(8) as usize;
        let tree_start = 28;
        let data_start = tree_start + tree_size;
        ensure!(data_start <= bytes.len(), "vpk tree exceeds file");
        let mut entries = HashMap::new();
        let mut pos = tree_start;
        loop {
            let ext = cstr(&bytes, &mut pos)?;
            if ext.is_empty() {
                break;
            }
            loop {
                let dir = cstr(&bytes, &mut pos)?;
                if dir.is_empty() {
                    break;
                }
                loop {
                    let name = cstr(&bytes, &mut pos)?;
                    if name.is_empty() {
                        break;
                    }
                    ensure!(pos + 18 <= bytes.len(), "truncated vpk entry");
                    let preload_len = u16::from_le_bytes([bytes[pos + 4], bytes[pos + 5]]) as usize;
                    let archive = u16::from_le_bytes([bytes[pos + 6], bytes[pos + 7]]);
                    let offset = u32_at(pos + 8);
                    let length = u32_at(pos + 12);
                    pos += 18;
                    let preload = bytes[pos..pos + preload_len].to_vec();
                    pos += preload_len;
                    let full = if dir == " " { format!("{name}.{ext}") } else { format!("{dir}/{name}.{ext}") };
                    entries.insert(full.to_lowercase(), Entry { preload, archive, offset, length });
                }
            }
        }
        Ok(Self { path, data_start, entries, bytes })
    }

    /// Contents of the entry at `path` (case-insensitive, forward slashes).
    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        let e = self.entries.get(&path.trim_start_matches('/').to_lowercase())?;
        let mut out = e.preload.clone();
        if e.length > 0 {
            let (off, len) = (e.offset as usize, e.length as usize);
            if e.archive == EMBEDDED {
                out.extend_from_slice(self.bytes.get(self.data_start + off..self.data_start + off + len)?);
            } else {
                let stem = self.path.file_stem()?.to_str()?;
                let base = stem.strip_suffix("_dir").unwrap_or(stem);
                let arc = self.path.with_file_name(format!("{base}_{:03}.vpk", e.archive));
                let d = std::fs::read(arc).ok()?;
                out.extend_from_slice(d.get(off..off + len)?);
            }
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_shipped_map_package() {
        let p = "E:/sbox-ref/game/core/maps/dev/preview_flat.vpk";
        let Ok(v) = Vpk::open(p) else { return };
        assert!(!v.entries.is_empty());
        let world = v.read("maps/dev/preview_flat/world.vwrld_c").expect("world");
        assert!(crate::Resource::parse(&world).is_ok());
    }
}
