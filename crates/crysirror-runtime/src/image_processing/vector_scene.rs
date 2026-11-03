//! One owner-polled vector raster job over frozen client image leases.
use super::*;
use crysirror_core::vector_scene::VectorSceneSpec;
pub(crate) fn begin(
    owner: ClientId,
    spec: VectorSceneSpec,
    pool: TaskPool,
    memory: Arc<AtomicUsize>,
) -> Result<ImageJob> {
    let budget = spec.validate()?;
    if spec.bitmaps.values().any(|image| image.client() != owner) {
        return Err(Error::Invalid("vector scene image owner"));
    }
    let length = crysirror_image::rgba_len(spec.size[0], spec.size[1])
        .map_err(|e| Error::Io(e.to_string()))?;
    let scratch = length
        .checked_mul(budget.depth + 4)
        .and_then(|n| n.checked_add(budget.bitmap_bytes))
        .and_then(|n| n.checked_add(budget.path_commands.checked_mul(64)?))
        .ok_or(Error::Limit("vector scene scratch bytes"))?;
    let output = Charge::new(memory.clone(), length)?;
    let scratch = Charge::new(memory, scratch)?;
    let task = pool
        .reserve()
        .map_err(|e| Error::Io(e.to_string()))?
        .spawn(move |cancel| {
            let _scratch = scratch;
            cancel.check()?;
            let image = crysirror_image::vector_scene::render(&spec, &|| cancel.check())?;
            resource_from_surface(owner, image, output, &|| cancel.check())
        })
        .map_err(|e| Error::Io(e.to_string()))?;
    Ok(Box::new(Job {
        task: Some(task),
        state: ImageProcessState::Pending,
    }))
}
