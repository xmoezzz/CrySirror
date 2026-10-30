//! Frozen group alpha is applied once after its children are composed.
//! This is engine-owned logical isolation; the renderer uses ordinary nested
//! immutable snapshot resources, without a VM rasterizer or CPU framebuffer.
use super::*;

pub(super) fn isolate(layers: &mut Vec<LayerSpec>) -> Result<()> {
    loop {
        let parents: BTreeSet<_> = layers.iter().filter_map(|layer| layer.parent).collect();
        let by_id: BTreeMap<_, _> = layers.iter().map(|layer| (layer.id, layer)).collect();
        let mut groups = Vec::new();
        for layer in layers
            .iter()
            .filter(|layer| layer.opacity != 255 && parents.contains(&layer.id))
        {
            let mut depth = 0;
            let mut parent = layer.parent;
            while let Some(id) = parent {
                depth += 1;
                ensure!(depth < 30, "snapshot isolated group depth budget");
                parent = by_id
                    .get(&id)
                    .context("snapshot isolated group parent missing")?
                    .parent;
            }
            groups.push((depth, layer.id));
        }
        let Some((_, id)) = groups.into_iter().max() else {
            return Ok(());
        };
        let root = by_id[&id].clone();
        let size = [root.rect[2].ceil() as u32, root.rect[3].ceil() as u32];
        ensure!(
            size.iter().all(|n| (1..=8192).contains(n)),
            "snapshot isolated group extent budget"
        );
        // The declared local clip bounds all child pixels. Refuse an unbounded
        // group instead of silently cropping descendants to the texture extent.
        let clip = root
            .clip
            .context("snapshot isolated group requires bounded local clip")?;
        ensure!(
            clip[0] >= 0.
                && clip[1] >= 0.
                && clip[0] + clip[2] <= size[0] as f32
                && clip[1] + clip[3] <= size[1] as f32,
            "snapshot isolated group clip exceeds local extent"
        );
        let selected = descendants_of(layers, id);
        let mut input: Vec<_> = layers
            .iter()
            .filter(|layer| selected.contains(&layer.id))
            .cloned()
            .collect();
        let input_root = input.iter_mut().find(|layer| layer.id == id).unwrap();
        input_root.parent = None;
        input_root.rect[0] = 0.;
        input_root.rect[1] = 0.;
        input_root.transform = SpriteTransform::default();
        input_root.opacity = 255;
        ensure!(
            snapshot_nesting(&input) < MAX_SNAPSHOT_NESTING,
            "snapshot isolated group nesting budget"
        );
        // Keep the source paint's UV/filter inside. The outer node transforms
        // the whole isolated surface and owns the original group alpha.
        let mut output = root;
        output.paint = Paint::Snapshot {
            size,
            layers: input.into(),
        };
        output.uv = None;
        output.sampling = ImageSampling::default();
        output.filter = None;
        layers.retain(|layer| !selected.contains(&layer.id));
        layers.push(output);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn specs() -> Vec<LayerSpec> {
        serde_json::from_value(serde_json::json!([
            {"id":1,"rect":[0,0,4,4],"paint":{"kind":"solid","color":[0,0,0,0]}},
            {"id":2,"parent":1,"rect":[0,0,4,4],"clip":[0,0,4,4],"opacity":128,"paint":{"kind":"solid","color":[0,0,0,0]}},
            {"id":3,"parent":2,"rect":[0,0,3,4],"paint":{"kind":"solid","color":[255,0,0,255]}},
            {"id":4,"parent":2,"rect":[1,0,3,4],"paint":{"kind":"solid","color":[255,0,0,255]}}
        ])).unwrap()
    }
    #[test]
    fn frozen_overlap_is_composed_before_group_alpha_and_keeps_source_state() {
        let mut system = LayerSystem::default();
        system.install(ClientId(1), &specs()).unwrap();
        let frozen = system.snapshot_subtree_specs(ClientId(1), 1).unwrap();
        assert_eq!(frozen.len(), 2);
        let group = frozen.iter().find(|l| l.id == 2).unwrap();
        assert_eq!(group.opacity, 128);
        let Paint::Snapshot { size, layers } = &group.paint else {
            panic!("group must be isolated")
        };
        assert_eq!(*size, [4, 4]);
        assert_eq!(layers.iter().find(|l| l.id == 2).unwrap().opacity, 255);
        assert!(
            layers
                .iter()
                .filter(|l| l.id == 3 || l.id == 4)
                .all(|l| l.opacity == 255)
        );
        validate_snapshot([4, 4], &frozen).unwrap();
        assert_eq!(system.get(ClientId(1), 2).unwrap().opacity, 128);
        assert!(system.get(ClientId(1), 4).is_some());
    }
    #[test]
    fn unbounded_group_is_rejected_instead_of_cropping_children() {
        let mut input = specs();
        input[1].clip = None;
        let mut system = LayerSystem::default();
        system.install(ClientId(1), &input).unwrap();
        assert!(
            system
                .snapshot_subtree_specs(ClientId(1), 1)
                .unwrap_err()
                .to_string()
                .contains("bounded local clip")
        );
    }
}
