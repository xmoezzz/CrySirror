//! BWEF relocation tables. Byte parsing and relocation do not depend on a VM.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OffsetTable {
    pub stride: u32,
    pub offsets: Vec<u32>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    Header,
    Length,
}
impl OffsetTable {
    pub fn parse(bytes: &[u8]) -> Result<Self, ParseError> {
        if bytes.len() < 288 || &bytes[..8] != b"bwef    " {
            return Err(ParseError::Header);
        }
        let count = u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize;
        if count > 4_194_304
            || count.checked_mul(4).and_then(|n| n.checked_add(288)) != Some(bytes.len())
        {
            return Err(ParseError::Length);
        }
        Ok(Self {
            stride: u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            offsets: bytes[288..]
                .chunks_exact(4)
                .map(|p| u32::from_le_bytes(p.try_into().unwrap()))
                .collect(),
        })
    }
    pub fn relocate(&self, base: u32) -> Vec<u8> {
        self.offsets
            .iter()
            .flat_map(|offset| [base.wrapping_add(*offset), self.stride])
            .flat_map(u32::to_le_bytes)
            .collect()
    }
}
