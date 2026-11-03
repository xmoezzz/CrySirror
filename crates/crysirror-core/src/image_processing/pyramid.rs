//! Immutable owner-scoped stored-byte mip planes.
use super::*;
#[derive(Clone, Debug)]
pub struct ImagePyramidLevel {
    pub size: [u32; 2],
    pub pixels: Arc<[u8]>,
}
#[derive(Clone, Debug)]
pub struct ImagePyramidResource {
    pub client: ClientId,
    pub levels: Vec<ImagePyramidLevel>,
}
impl ImagePyramidResource {
    pub fn validate(&self, owner: ClientId) -> Result<()> {
        if self.client != owner {
            return Err(crate::Error::Invalid("foreign image pyramid"));
        }
        if self.levels.is_empty() || self.levels.len() > 5 {
            return Err(crate::Error::Limit("image pyramid levels"));
        }
        for (index, level) in self.levels.iter().enumerate() {
            let [w, h] = level.size;
            let n = u64::from(w) * u64::from(h);
            if w == 0
                || h == 0
                || w > 16384
                || h > 16384
                || n > 4_194_304
                || n * 4 != level.pixels.len() as u64
            {
                return Err(crate::Error::Limit("image pyramid plane"));
            }
            if index > 0 {
                let previous = self.levels[index - 1].size;
                if level.size != [previous[0].div_ceil(2), previous[1].div_ceil(2)] {
                    return Err(crate::Error::Invalid("image pyramid plane dimensions"));
                }
            }
        }
        Ok(())
    }
    pub fn byte_len(&self) -> usize {
        self.levels.iter().map(|v| v.pixels.len()).sum()
    }
}
