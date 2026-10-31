// SPDX-License-Identifier: MPL-2.0
//! Native filename/TCC policy only. Format facts cross-checked against complete
//! RLVM e38cda7783dc67539ce27901596ed93a2bb5c826 tone_curve.cc and
//! sdl_graphics_system.cc; siglus_rs 5f358ef tone_curve.rs/resource.rs. No
//! legacy pixel implementation is copied: shared RgbLookup owns all RGB math.
use super::*;
use crate::vm::graphics::Image;
use crysirror_core::image_processing::{ImageInput, ImageJob, ImageProcessState};
use crysirror_image::{
    effects::ImageEffect,
    rgb_lut::{RgbLookup, RgbLookupProfile},
};
use crysirror_tasks::Task;
const TONE_SLOT_BASE: u32 = 0x2400_0000;
const MAX_TONE_IMAGES: usize = 32;
const MAX_TONE_BYTES: u64 = 128 * 1024 * 1024;
struct Prepared {
    image: Image,
    effect: RgbLookup,
    fingerprint: [u8; 32],
    source: Arc<[u8]>,
}
enum Stage {
    Metadata(Task<Prepared>),
    Pixels { image: Image, job: ImageJob },
}
pub(super) struct PendingTone {
    key: String,
    stage: Stage,
}
#[derive(Default)]
pub(super) struct ToneImages {
    pub ready: BTreeMap<String, Rc<Image>>,
    bytes: u64,
    next: u32,
    source: Option<Arc<[u8]>>,
}
/// RLVM's suffix is decimal effect/10-1, not a G00 pattern index. Invalid
/// numerical encodings fail explicitly rather than losing the requested effect.
pub(super) fn request(name: &str) -> Result<Option<(&str, usize)>> {
    let Some((base, suffix)) = name.split_once('?') else {
        return Ok(None);
    };
    ensure!(name.len() <= 4096, "RealLive tone filename byte budget");
    ensure!(
        !base.is_empty() && !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()),
        "invalid RealLive tone-curve filename suffix"
    );
    let n = suffix
        .parse::<i32>()
        .context("tone-curve suffix outside native integer range")?;
    ensure!(n >= 10, "RealLive tone-curve suffix must be >=10");
    Ok(Some((base, (n / 10 - 1) as usize)))
}
fn table(bytes: &[u8], index: usize) -> Result<RgbLookup> {
    ensure!(
        bytes.len() >= 8 && bytes.len() <= 1024 * 1024,
        "TCC byte budget/header"
    );
    ensure!(
        i32::from_le_bytes(bytes[..4].try_into().unwrap()) == 1000,
        "invalid TCC version"
    );
    let count = i32::from_le_bytes(bytes[4..8].try_into().unwrap());
    ensure!((0..=256).contains(&count), "TCC table count budget");
    ensure!(
        index < count as usize,
        "RealLive tone-curve table index out of range"
    );
    let required = 0xfa8usize
        .checked_add(count as usize * 0x340)
        .context("TCC length overflow")?;
    ensure!(bytes.len() >= required, "truncated TCC table records");
    let at = 0xfe8 + index * 0x340;
    Ok(RgbLookup {
        red: bytes[at..at + 256].to_vec(),
        green: bytes[at + 256..at + 512].to_vec(),
        blue: bytes[at + 512..at + 768].to_vec(),
        profile: RgbLookupProfile::StraightRgba,
    })
}
impl RealLiveVm {
    pub(super) fn prepare_tone_image(
        &mut self,
        m: &Machine,
        h: &mut dyn HostServices,
        name: &str,
    ) -> Result<bool> {
        let Some((base, index)) = request(name.trim())? else {
            return Ok(true);
        };
        let key = name.trim().to_lowercase();
        let assets = self
            .object_assets
            .as_ref()
            .context("RealLive object assets")?
            .clone();
        if self.tone_images.ready.contains_key(&key) {
            return Ok(true);
        }
        if let Some(p) = &self.pending_tone {
            ensure!(p.key == key, "RealLive tone request changed while pending");
        }
        if self.pending_tone.is_none() {
            ensure!(
                self.tone_images.ready.len() < MAX_TONE_IMAGES,
                "RealLive transformed image count budget"
            );
            let mut r = assets.resources.borrow_mut();
            let path = r.find(&m.gameexe, "image", base)?;
            let prior_source = self.tone_images.source.clone();
            let tone_path = if prior_source.is_none() {
                let tcc = m
                    .gameexe
                    .str("TONECURVE_FILENAME")
                    .filter(|s| !s.is_empty())
                    .context("tone suffix requires #TONECURVE_FILENAME")?
                    .replace('\\', "/");
                let (sub, file) = tcc.rsplit_once('/').unwrap_or(("", &tcc));
                let dir = if sub.is_empty() {
                    "dat".to_owned()
                } else {
                    format!("dat/{sub}")
                };
                Some(
                    r.file(&dir, file)?
                        .context("configured native TCC absent")?,
                )
            } else {
                None
            };
            let storage = r.storage.clone();
            drop(r);
            let metadata = assets.metadata.clone();
            let task = self.tone_pool.spawn(move |cancel| {
                cancel.check()?;
                // The original tone bank is an immutable game-global snapshot;
                // later variants reuse it even if a loose file changes/disappears.
                let source = if let Some(bytes) = prior_source {
                    bytes
                } else {
                    let reader = storage
                        .open_asset(&VfsPath::new(Mount::Asset, tone_path.as_ref().unwrap())?)?;
                    Arc::<[u8]>::from(crysirror_resource::read_all(reader.as_ref(), 1024 * 1024)?)
                };
                let effect = table(&source, index)?;
                cancel.check()?;
                let reader = storage.open_asset(&VfsPath::new(Mount::Asset, &path)?)?;
                let bytes = crysirror_resource::read_all(reader.as_ref(), 64 * 1024 * 1024)?;
                cancel.check()?;
                let fingerprint = *blake3::hash(&bytes).as_bytes();
                let (width, height, regions) = if path.to_ascii_lowercase().ends_with(".g00") {
                    ensure!(
                        bytes.len() >= 5
                            && u16::from_le_bytes(bytes[1..3].try_into().unwrap()) > 0
                            && u16::from_le_bytes(bytes[3..5].try_into().unwrap()) > 0,
                        "invalid tone G00 dimensions"
                    );
                    if bytes[0] == 2 {
                        ensure!(bytes.len() >= 9, "truncated tone G00 metadata");
                        let n = u32::from_le_bytes(bytes[5..9].try_into().unwrap());
                        ensure!((1..=4096).contains(&n), "tone G00 region count budget");
                    }
                    crysirror_image::g00::g00_regions(&bytes)?
                } else {
                    let size = metadata.size(&path)?;
                    let (w, h) = (i32::try_from(size[0])?, i32::try_from(size[1])?);
                    (w, h, vec![crysirror_image::g00::Region::full(w, h)])
                };
                ensure!(
                    width > 0
                        && height > 0
                        && width as u64 * height as u64
                            <= crysirror_image::effects::MAX_EFFECT_PIXELS,
                    "RealLive tone image dimensions/budget"
                );
                cancel.check()?;
                Ok(Prepared {
                    image: Image {
                        asset: path,
                        width,
                        height,
                        regions,
                    },
                    effect,
                    fingerprint,
                    source,
                })
            })?;
            self.pending_tone = Some(PendingTone {
                key: key.clone(),
                stage: Stage::Metadata(task),
            });
        }
        let pending = self.pending_tone.as_mut().unwrap();
        if let Stage::Metadata(task) = &mut pending.stage {
            let Some(prepared) = task.try_take()? else {
                return Ok(false);
            };
            self.tone_images.source = Some(prepared.source);
            let size = [prepared.image.width as u32, prepared.image.height as u32];
            let effects = serde_json::to_vec(&[ImageEffect::RgbLookup {
                spec: prepared.effect,
            }])?;
            let job = assets
                .metadata
                .begin_process(
                    ImageInput::Asset {
                        path: prepared.image.asset.clone(),
                        size,
                        fingerprint: Some(prepared.fingerprint),
                    },
                    effects.into(),
                )
                .map_err(err)?;
            pending.stage = Stage::Pixels {
                image: prepared.image,
                job,
            };
        }
        let Stage::Pixels { image, job } = &mut pending.stage else {
            unreachable!()
        };
        let resource = match job.poll().map_err(err)? {
            ImageProcessState::Pending => return Ok(false),
            ImageProcessState::Failed(e) => bail!("shared RealLive tone image failed: {e}"),
            ImageProcessState::Ready(r) => r,
        };
        ensure!(
            resource.client() == self.client,
            "foreign transformed RealLive image"
        );
        let size = resource.size();
        ensure!(
            size == [image.width as u32, image.height as u32],
            "transformed RealLive geometry changed"
        );
        let bytes = u64::from(size[0]) * u64::from(size[1]) * 4;
        ensure!(
            self.tone_images
                .bytes
                .checked_add(bytes)
                .is_some_and(|n| n <= MAX_TONE_BYTES),
            "RealLive tone cache byte budget"
        );
        let id = TONE_SLOT_BASE
            .checked_add(self.tone_images.next)
            .context("tone image id exhausted")?;
        // Sole publication point: all file reads, LUT validation and pixel work have
        // completed successfully. Every variant gets an immutable distinct path.
        let published = h
            .publish_image_resource(self.client, id, resource)
            .map_err(err)?;
        let mut ready = image.clone();
        ready.asset = published.path.relative().into();
        let ready = Rc::new(ready);
        assets
            .images
            .borrow_mut()
            .insert(key.clone(), ready.clone());
        self.tone_images.ready.insert(key, ready);
        self.tone_images.bytes += bytes;
        self.tone_images.next += 1;
        self.pending_tone = None;
        Ok(true)
    }
    pub(super) fn object_creation(
        &mut self,
        m: &mut Machine,
        h: &mut dyn HostServices,
        c: &Command,
    ) -> Result<Next> {
        let name = crate::vm::modules::obj::creation_image_name(m, c)?;
        if let Some(name) = name {
            if let Some(image) = self.tone_images.ready.get(&name.trim().to_lowercase()) {
                self.object_assets
                    .as_ref()
                    .unwrap()
                    .images
                    .borrow_mut()
                    .insert(name.trim().to_lowercase(), image.clone());
            }
            if !self.prepare_tone_image(m, h, &name)? {
                self.image_pending = true;
                return Ok(Next::Jumped);
            }
        }
        self.image_pending = false;
        crate::vm::modules::obj::creation(m, c)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_tone_suffix_and_tcc_select_original_table_without_pattern_alias() {
        let mut bytes = vec![0u8; 0xfa8 + 3 * 0x340];
        bytes[..4].copy_from_slice(&1000i32.to_le_bytes());
        bytes[4..8].copy_from_slice(&3i32.to_le_bytes());
        for i in 0..3 {
            let at = 0xfe8 + i * 0x340;
            for n in 0..768 {
                bytes[at + n] = (i * 37 + n) as u8;
            }
        }
        let (base, index) = request("CGMS12?030").unwrap().unwrap();
        assert_eq!((base, index), ("CGMS12", 2));
        let curve = table(&bytes, index).unwrap();
        assert_eq!(curve.red[0], 74);
        assert_eq!(curve.green[255], 73);
        assert_eq!(request("x?039").unwrap().unwrap().1, 2);
        assert!(table(&bytes, 3).is_err());
        assert!(table(&bytes[..bytes.len() - 1], 2).is_err());
        for name in ["?030", "x?", "x?009", "x?030?1", "x?abc"] {
            assert!(request(name).is_err());
        }
    }
}
