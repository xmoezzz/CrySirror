// SPDX-License-Identifier: MPL-2.0
//! Narrow, owned definitions from the Midori 2011 def.rld prelude.
use crate::branch::number;
use anyhow::{Result, bail, ensure};
use crysirror_core::{
    image_processing::{ImageInput, ImageJob, ImageProcessState, ImageResource},
    path::{Mount, VfsPath},
    service::HostServices,
    state::ClientId,
};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DebugDefinition {
    pub mode: u32,
    pub pairs: [[i32; 2]; 8],
}

impl DebugDefinition {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        // The packed retail definition has 22 literal numeric fields. Its
        // high bits do not alter liteSetDebugMode's low-28-bit mode.
        let words = raw
            .split(|&b| b == b',')
            .map(number)
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            words.len() == 22,
            "unsupported Midori debug definition size"
        );
        let mode = words[0] as u32 & 0x0fff_ffff;
        ensure!(
            mode == 0 && words[1..6] == [-1, 0, 0, 0, 0],
            "unsupported Midori debug overlay"
        );
        let pairs = std::array::from_fn(|i| [words[6 + i * 2], words[7 + i * 2]]);
        Ok(Self { mode, pairs })
    }
}

const MARKER_NATIVE: &str = r"res\g\sy\01\01_950.gyu";
const MARKER_ASSET: &str = "res/g/sy/01/01_950.gyu";
const MARKER_SEQUENCE: &str = "A.B.C.D.E.F.G.H.";
const MARKER_SIZE: [u32; 2] = [240, 30];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MarkerDefinition {
    fixed: bool,
    anchor: [i32; 2],
}

impl MarkerDefinition {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        let fields: Vec<_> = raw.splitn(10, |b| *b == b',').collect();
        ensure!(
            fields.len() == 9
                && fields[..3] == [b"60".as_slice(), b"-1", b"6"]
                && fields[6..]
                    == [
                        b"8".as_slice(),
                        MARKER_NATIVE.as_bytes(),
                        MARKER_SEQUENCE.as_bytes()
                    ],
            "unsupported Midori hit-wait marker"
        );
        let fixed = match fields[3] {
            b"0" => false,
            b"1" => true,
            _ => bail!("unsupported Midori hit-wait positioning flags"),
        };
        let anchor = [number(fields[4])?, number(fields[5])?];
        Ok(Self { fixed, anchor })
    }

    fn image_input(&self) -> Result<ImageInput> {
        Ok(ImageInput::ScopedAsset {
            path: VfsPath::new(Mount::Asset, MARKER_ASSET)?,
            size: MARKER_SIZE,
            fingerprint: None,
        })
    }

    #[cfg(test)]
    pub fn stock_input(
        &self,
        storage: &dyn crysirror_core::service::ClientStorage,
    ) -> Result<ImageInput> {
        let ImageInput::ScopedAsset { path, size, .. } = self.image_input()? else {
            unreachable!()
        };
        let reader = storage.open_read(&path)?;
        let bytes = crysirror_resource::read_all(reader.as_ref(), 1 << 20)?;
        Ok(ImageInput::Encoded {
            name: path.relative().to_owned(),
            bytes: Arc::from(bytes),
            size,
        })
    }

    // RetouchPrintManager::saveMark (0x420e0). Mode zero clears both
    // configured position words; the file/program still belong to the atlas.
    fn capture(&self) -> (u32, String) {
        let [x, y] = if self.fixed { self.anchor } else { [0, 0] };
        (
            202,
            format!(
                "0;{},{x},{y},8,{MARKER_NATIVE},{MARKER_SEQUENCE}",
                u8::from(self.fixed)
            ),
        )
    }
}

#[derive(Default)]
pub(super) struct Marker {
    active: Option<(MarkerDefinition, Arc<dyn ImageResource>)>,
    pending: Option<(MarkerDefinition, ImageJob)>,
}

impl Marker {
    pub fn begin(
        &mut self,
        def: MarkerDefinition,
        host: &mut dyn HostServices,
        client: ClientId,
    ) -> Result<()> {
        ensure!(!self.preparing(), "overlapping Midori hit-wait marker");
        let input = def.image_input()?;
        let job = host
            .image_metadata(client)?
            .begin_process(input, Arc::from(&b"[]"[..]))?;
        self.pending = Some((def, job));
        Ok(())
    }

    pub fn preparing(&self) -> bool {
        self.pending.is_some()
    }

    pub fn poll(&mut self, client: ClientId) -> Result<bool> {
        let Some((def, mut job)) = self.pending.take() else {
            return Ok(true);
        };
        match job.poll()? {
            ImageProcessState::Pending => {
                self.pending = Some((def, job));
                Ok(false)
            }
            ImageProcessState::Failed(error) => bail!("Midori hit-wait marker atlas: {error}"),
            ImageProcessState::Ready(image) => {
                ensure!(
                    image.client() == client && image.size() == MARKER_SIZE,
                    "invalid Midori hit-wait marker image"
                );
                self.active = Some((def, image));
                Ok(true)
            }
        }
    }

    pub fn capture(&self) -> Option<(u32, String)> {
        if self.preparing() {
            return None;
        }
        self.active
            .as_ref()
            .map(|(definition, _)| definition.capture())
    }

    pub fn geometry(&self) -> Option<[u32; 2]> {
        self.active.as_ref().map(|(_, image)| image.size())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crysirror_core::image_processing::{ImageProcess, RgbaImage};

    struct Job(Vec<ImageProcessState>);
    impl ImageProcess for Job {
        fn poll(&mut self) -> crysirror_core::Result<ImageProcessState> {
            Ok(self.0.remove(0))
        }
    }

    #[test]
    fn marker_waits_then_commits_and_failed_replacement_preserves_source_and_capture() -> Result<()>
    {
        let mut marker = Marker::default();
        let fixed = MarkerDefinition::parse(
            br"60,-1,6,1,980,542,8,res\g\sy\01\01_950.gyu,A.B.C.D.E.F.G.H.",
        )?;
        let relative =
            MarkerDefinition::parse(br"60,-1,6,0,23,-5,8,res\g\sy\01\01_950.gyu,A.B.C.D.E.F.G.H.")?;
        let client = ClientId(7);
        let make = |c| -> Result<Arc<dyn ImageResource>> {
            Ok(Arc::new(RgbaImage::new(
                c,
                MARKER_SIZE,
                Arc::from(vec![0; 4 * 240 * 30]),
            )?))
        };
        marker.pending = Some((
            fixed.clone(),
            Box::new(Job(vec![
                ImageProcessState::Pending,
                ImageProcessState::Ready(make(client)?),
            ])),
        ));
        assert!(marker.capture().is_none());
        assert!(!marker.poll(client)?);
        assert!(marker.capture().is_none());
        assert!(marker.poll(client)?);
        assert_eq!(marker.geometry(), Some([240, 30]));
        assert_eq!(
            marker.capture(),
            Some((
                202,
                r"0;1,980,542,8,res\g\sy\01\01_950.gyu,A.B.C.D.E.F.G.H.".into()
            ))
        );
        for failure in [
            ImageProcessState::Failed("bad atlas".into()),
            ImageProcessState::Ready(make(ClientId(8))?),
        ] {
            marker.pending = Some((relative.clone(), Box::new(Job(vec![failure]))));
            assert!(marker.capture().is_none());
            assert!(marker.poll(client).is_err());
            assert_eq!(marker.capture(), Some(fixed.capture()));
        }
        marker.pending = Some((
            relative,
            Box::new(Job(vec![ImageProcessState::Ready(make(client)?)])),
        ));
        assert!(marker.poll(client)?);
        assert_eq!(
            marker.capture(),
            Some((
                202,
                r"0;0,0,0,8,res\g\sy\01\01_950.gyu,A.B.C.D.E.F.G.H.".into()
            ))
        );
        Ok(())
    }
}
