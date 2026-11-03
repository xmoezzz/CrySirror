//! Shared multi-frame codec jobs. The owner publishes only completed leases.
use super::*;
use crysirror_core::frame_image::{EncodedFrameFormat, EncodedFrameSpec};
pub(crate) fn begin(
    owner: ClientId,
    spec: EncodedFrameSpec,
    pool: TaskPool,
    memory: Arc<AtomicUsize>,
) -> Result<ImageJob> {
    if spec.destination.client() != owner {
        return Err(Error::Invalid("frame image owner"));
    }
    let [w, h] = spec.destination.size();
    let length = crysirror_image::rgba_len(w, h).map_err(|e| Error::Io(e.to_string()))?;
    if length > 64 * 1024 * 1024 || spec.bytes.len() > 64 * 1024 * 1024 {
        return Err(Error::Limit("frame image bytes"));
    }
    let padded = u64::from(w.saturating_add(7) & !7) * u64::from(h.saturating_add(7) & !7) * 4;
    let padded = usize::try_from(padded).map_err(|_| Error::Limit("frame image padded bytes"))?;
    let output = Charge::new(memory.clone(), length)?;
    let scratch = Charge::new(
        memory,
        length
            .checked_add(padded)
            .and_then(|n| n.checked_add(spec.bytes.len()))
            .ok_or(Error::Limit("frame image scratch"))?,
    )?;
    let task = pool
        .reserve()
        .map_err(|e| Error::Io(e.to_string()))?
        .spawn(move |cancel| {
            let _scratch = scratch;
            cancel.check()?;
            let mut image = crysirror_image::cbg::DecodedImage {
                width: w,
                height: h,
                rgba: spec.destination.rgba().to_vec(),
            };
            match spec.format {
                EncodedFrameFormat::BfMovie => {
                    crysirror_image::cbg::decode_bf_movie_frame(&spec.bytes, spec.frame, &mut image)
                        .map_err(|e| anyhow::anyhow!("{e}"))?
                }
            }
            cancel.check()?;
            resource_from_surface(
                owner,
                ImageSurface {
                    width: image.width,
                    height: image.height,
                    rgba: image.rgba,
                },
                output,
                &|| cancel.check(),
            )
        })
        .map_err(|e| Error::Io(e.to_string()))?;
    Ok(Box::new(Job {
        task: Some(task),
        state: ImageProcessState::Pending,
    }))
}
