// SPDX-License-Identifier: MPL-2.0
//! Checked binary reads shared by format frontends. No format/opcode policy.
use crate::{Error, Result};

pub fn bytes_at(data: &[u8], offset: usize, length: usize) -> Result<&[u8]> {
    let end = offset
        .checked_add(length)
        .ok_or(Error::Invalid("binary range overflow"))?;
    data.get(offset..end)
        .ok_or(Error::Invalid("binary range out of bounds"))
}
pub fn u16_le(data: &[u8], offset: usize) -> Result<u16> {
    let b = bytes_at(data, offset, 2)?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}
pub fn u32_le(data: &[u8], offset: usize) -> Result<u32> {
    let b = bytes_at(data, offset, 4)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}
pub fn i32_le(data: &[u8], offset: usize) -> Result<i32> {
    Ok(u32_le(data, offset)? as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_and_endianness() {
        assert_eq!(u32_le(&[0x78, 0x56, 0x34, 0x12], 0).unwrap(), 0x12345678);
        assert_eq!(i32_le(&[255; 4], 0).unwrap(), -1);
        assert!(u16_le(&[0], 0).is_err());
        assert!(bytes_at(&[], usize::MAX, 1).is_err());
        assert_eq!(bytes_at(&[], 0, 0).unwrap(), &[]);
    }
}
