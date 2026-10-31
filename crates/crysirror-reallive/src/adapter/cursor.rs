// SPDX-License-Identifier: MPL-2.0
//! RealLive cursor metadata policy over shared image/UI cursor capabilities.
//! Layout reference (format facts, no copied implementation): RLVM
//! e38cda7783dc67539ce27901596ed93a2bb5c826, complete mouse_cursor.{cc,h}
//! and graphics_system.cc. The VM never renders or clocks pointer pixels.
use super::*;
use crysirror_core::image_processing::{ImageInput, ImageJob, ImageProcessState};
use crysirror_image::{
    ImageSurface,
    cursor::{SpriteCursorLayout, SpriteHotspot, cursor_from_sprite, sprite_source_size},
};

pub(super) struct PendingCursor {
    number: i32,
    layout: SpriteCursorLayout,
    job: ImageJob,
}

impl RealLiveVm {
    /// A pending image preparation fences native execution. Workers own encoded
    /// resource reads and decoding; only a ready immutable receipt reaches UI.
    pub(super) fn sync_cursor(&mut self, m: &Machine, host: &mut dyn HostServices) -> Result<bool> {
        let number = m.sys.mouse_cursor;
        if self.published_cursor == Some(number) {
            self.pending_cursor = None;
            return Ok(true);
        }
        if self
            .pending_cursor
            .as_ref()
            .is_some_and(|p| p.number != number)
        {
            self.pending_cursor = None;
        }
        let prefix = format!("MOUSE_CURSOR.{number:03}");
        let name = m
            .gameexe
            .str(&format!("{prefix}.NAME"))
            .filter(|s| !s.is_empty());
        let mut ops = Vec::new();
        let custom = m.gameexe.get("MOUSE_CURSOR").is_some() && name.is_some();
        let selection = if custom {
            let id = u32::try_from(number).context("negative custom mouse cursor id")?;
            if !self.cursor_definitions.contains(&number) {
                if self.pending_cursor.is_none() {
                    let count = m.gameexe.int(&format!("{prefix}.CONT")).unwrap_or(1);
                    ensure!(
                        (1..=256).contains(&count),
                        "RealLive mouse cursor frame count"
                    );
                    let period = m.gameexe.int(&format!("{prefix}.SPEED")).unwrap_or(800);
                    ensure!(
                        period >= count || count == 1,
                        "RealLive mouse cursor animation period"
                    );
                    let path = self.resource(m, "image", name.unwrap())?;
                    let storage = self
                        .resources
                        .as_ref()
                        .context("RealLive resources")?
                        .borrow()
                        .storage
                        .clone();
                    let reader = storage
                        .open_read(&VfsPath::new(Mount::Asset, &path)?)
                        .map_err(err)?;
                    let length = reader.len().map_err(err)?;
                    let mut header = vec![0u8; length.min(32) as usize];
                    read_header(&*reader, &mut header, 0)?;
                    if path.to_ascii_lowercase().ends_with(".g00") && header.first() == Some(&2) {
                        ensure!(header.len() >= 9, "truncated cursor G00 header");
                        let regions = u32::from_le_bytes(header[5..9].try_into().unwrap());
                        ensure!(
                            (1..=256).contains(&regions),
                            "cursor G00 region table budget"
                        );
                        let needed = 9 + regions as usize * 24;
                        ensure!(length >= needed as u64, "truncated cursor G00 region table");
                        let old = header.len();
                        header.resize(needed, 0);
                        read_header(&*reader, &mut header[old..], old as u64)?;
                    }
                    let size = sprite_source_size(&path, &header)?;
                    ensure!(u64::from(size[0]) * u64::from(size[1]) <= 1024 * 1024, "cursor source pixel budget");
                    // An empty composite is the existing shared no-effect
                    // load/copy profile; it still validates source size and
                    // reserves worker and decoded-resource memory budgets.
                    let input = ImageInput::Composite {
                        base: Box::new(ImageInput::Asset {
                            path,
                            size,
                            fingerprint: None,
                        }),
                        layers: vec![],
                    };
                    let job = host
                        .image_metadata(self.client)
                        .map_err(err)?
                        .begin_process(input, Arc::from(&b"[]"[..]))
                        .map_err(err)?;
                    self.pending_cursor = Some(PendingCursor {
                        number,
                        job,
                        layout: SpriteCursorLayout {
                            frames: (0..count as u32).map(|i| [8 + i * 32, 8, 32, 32]).collect(),
                            hotspot: SpriteHotspot::Marker {
                                rect: [8, 48, 32, 32],
                                rgb: [255; 3],
                                fallback: [0, 0],
                            },
                            color_key_at: Some([0, 0]),
                            frame_ms: (period.max(1) / count) as u32,
                        },
                    });
                }
                let pending = self
                    .pending_cursor
                    .as_mut()
                    .expect("cursor preparation admitted");
                let resource = match pending.job.poll().map_err(err)? {
                    ImageProcessState::Pending => return Ok(false),
                    ImageProcessState::Failed(message) => {
                        bail!("shared cursor image preparation failed: {message}")
                    }
                    ImageProcessState::Ready(resource) => resource,
                };
                ensure!(
                    resource.client() == self.client,
                    "cursor image owner mismatch"
                );
                // At most 256 x 32 x 32 output texels. The small shared sprite
                // kernel is owner work; decoding and all full-file reads are
                // completed by the shared worker before this publication.
                let size = resource.size();
                ensure!(
                    u64::from(size[0]) * u64::from(size[1]) <= 1024 * 1024,
                    "cursor source pixel budget"
                );
                let image = ImageSurface {
                    width: size[0],
                    height: size[1],
                    rgba: resource.rgba().to_vec(),
                };
                let cursor = cursor_from_sprite(&image, &pending.layout)
                    .with_context(|| format!("RealLive mouse cursor {number}"))?;
                let frames:Vec<_>=cursor.frames.iter().map(|f|json!({"width":f.image.width,"height":f.image.height,"hotspot":f.hotspot,"rgba":f.image.rgba})).collect();
                let steps: Vec<_> = cursor.steps.iter().map(|&(i, ms)| [i as u32, ms]).collect();
                ops.push(
                    json!({"op":"define_cursor","id":id,"cursor":{"frames":frames,"steps":steps}}),
                );
            }
            json!({"kind":"custom","id":id})
        } else {
            json!({"kind":"standard","shape":"default"})
        };
        ops.push(json!({"op":"set_cursor","cursor":selection}));
        self.submit(host, json!({"ui":ops}))?;
        if custom {
            self.cursor_definitions.insert(number);
        }
        self.pending_cursor = None;
        self.published_cursor = Some(number);
        Ok(true)
    }
}

fn read_header(
    reader: &dyn crysirror_core::service::AssetReader,
    header: &mut [u8],
    offset: u64,
) -> Result<()> {
    let mut done = 0;
    while done < header.len() {
        let count = reader
            .read_at(offset + done as u64, &mut header[done..])
            .map_err(err)?;
        ensure!(count > 0, "truncated cursor resource header");
        done += count;
    }
    Ok(())
}
