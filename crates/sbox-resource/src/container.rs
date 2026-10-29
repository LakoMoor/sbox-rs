use anyhow::{bail, ensure, Result};

#[derive(Clone, Debug)]
pub struct Block {
    pub kind: [u8; 4],
    /// Absolute byte range inside the file.
    pub start: usize,
    pub len: usize,
}

impl Block {
    pub fn name(&self) -> String {
        String::from_utf8_lossy(&self.kind).into_owned()
    }
}

/// Header of a compiled resource file: file size, header version, resource version, block table.
pub struct Resource<'a> {
    pub data: &'a [u8],
    pub header_version: u16,
    pub version: u16,
    pub blocks: Vec<Block>,
}

fn u32_at(d: &[u8], o: usize) -> Result<u32> {
    ensure!(o + 4 <= d.len(), "truncated at {o}");
    Ok(u32::from_le_bytes(d[o..o + 4].try_into().unwrap()))
}

impl<'a> Resource<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        ensure!(data.len() >= 16, "file too small");
        let file_size = u32_at(data, 0)? as usize;
        ensure!(file_size <= data.len(), "size mismatch: header {file_size}, actual {}", data.len());
        let header_version = u16::from_le_bytes([data[4], data[5]]);
        if header_version != 12 {
            bail!("unsupported header version {header_version}");
        }
        let version = u16::from_le_bytes([data[6], data[7]]);
        // Offsets are relative to the position of the offset field itself.
        let table = 8 + u32_at(data, 8)? as usize;
        let count = u32_at(data, 12)? as usize;
        let mut blocks = Vec::with_capacity(count);
        for i in 0..count {
            let e = table + i * 12;
            ensure!(e + 12 <= data.len(), "block table truncated");
            let kind: [u8; 4] = data[e..e + 4].try_into().unwrap();
            let start = e + 4 + u32_at(data, e + 4)? as usize;
            let len = u32_at(data, e + 8)? as usize;
            ensure!(start + len <= data.len(), "block {} out of range", String::from_utf8_lossy(&kind));
            blocks.push(Block { kind, start, len });
        }
        Ok(Self { data, header_version, version, blocks })
    }

    pub fn block(&self, kind: &str) -> Option<&Block> {
        self.blocks.iter().find(|b| &b.kind == kind.as_bytes())
    }

    pub fn block_data(&self, kind: &str) -> Option<&'a [u8]> {
        self.block(kind).map(|b| &self.data[b.start..b.start + b.len])
    }
}
