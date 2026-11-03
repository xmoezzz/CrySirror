//! Retain bitmap operation order without keeping pixels a complete copy replaces.
use super::{ImageCompositeLayer, ImageCompositeMath, ImageInput};
use alloc::{boxed::Box, vec};

impl ImageInput {
    /// Append an immutable bitmap operation, discarding the destination history
    /// only when a full-opacity four-channel copy provably replaces every pixel.
    /// RGB/alpha-only copies, partial clips and out-of-source rectangles retain it.
    pub fn with_composite_layer(self, layer: ImageCompositeLayer) -> Self {
        let (Some(size), Some(source_size)) = (extent(&self), extent(&layer.input)) else {
            return append(self, layer);
        };
        let [sx, sy, width, height] = layer.source;
        let [dx, dy] = layer.destination;
        let full_copy = matches!(layer.math, ImageCompositeMath::Copy)
            && layer.opacity == 255
            && dx <= 0
            && dy <= 0
            && i64::from(dx) + i64::from(width) >= i64::from(size[0])
            && i64::from(dy) + i64::from(height) >= i64::from(size[1])
            && sx >= 0
            && sy >= 0
            && width > 0
            && height > 0
            && i64::from(sx) + i64::from(width) <= i64::from(source_size[0])
            && i64::from(sy) + i64::from(height) <= i64::from(source_size[1])
            && layer.clip.is_none_or(|[x, y, w, h]| {
                x <= 0
                    && y <= 0
                    && i64::from(x) + i64::from(w) >= i64::from(size[0])
                    && i64::from(y) + i64::from(h) >= i64::from(size[1])
            });
        if full_copy {
            if layer.source == [0, 0, size[0] as i32, size[1] as i32]
                && layer.destination == [0, 0]
                && source_size == size
            {
                return *layer.input;
            }
            // A copied atlas cell can cover the entire destination too. Keep
            // its original coordinates but release the overwritten history.
            return Self::Composite {
                base: Box::new(Self::Solid {
                    size,
                    color: [0; 4],
                }),
                layers: vec![layer],
            };
        }
        append(self, layer)
    }
}

fn append(base: ImageInput, layer: ImageCompositeLayer) -> ImageInput {
    match base {
        ImageInput::Composite { base, mut layers } if layers.len() < 64 => {
            layers.push(layer);
            ImageInput::Composite { base, layers }
        }
        base => ImageInput::Composite {
            base: Box::new(base),
            layers: vec![layer],
        },
    }
}

fn extent(input: &ImageInput) -> Option<[u32; 2]> {
    match input {
        ImageInput::ParticlePlane { size, .. }
        | ImageInput::Encoded { size, .. }
        | ImageInput::Solid { size, .. }
        | ImageInput::Asset { size, .. } => Some(*size),
        ImageInput::Composite { base, .. } => extent(base),
        ImageInput::Raster { destination, .. } | ImageInput::Ternary { destination, .. } => {
            extent(destination)
        }
        ImageInput::Resource(resource) => Some(resource.size()),
        ImageInput::Transition { from, .. } => Some(from.size()),
        // New worker graph kinds may change dimensions. Conservatively keep
        // their history rather than interpreting effect programs in core.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn solid(color: u8) -> ImageInput {
        ImageInput::Solid {
            size: [2, 2],
            color: [color; 4],
        }
    }
    fn copy(input: ImageInput) -> ImageCompositeLayer {
        ImageCompositeLayer {
            input: Box::new(input),
            source: [0, 0, 2, 2],
            destination: [0, 0],
            clip: None,
            opacity: 255,
            math: ImageCompositeMath::Copy,
        }
    }
    #[test]
    fn worker_effect_graph_keeps_history_without_guessing_its_output_size() {
        let source = ImageInput::Process {
            input: Box::new(solid(2)),
            effects: alloc::sync::Arc::from(&b"[]"[..]),
        };
        let result = solid(1).with_composite_layer(copy(source));
        let ImageInput::Composite { base, .. } = result else {
            panic!("lost destination history")
        };
        assert!(matches!(
            *base,
            ImageInput::Solid {
                color: [1, 1, 1, 1],
                ..
            }
        ));
    }

    #[test]
    fn repeated_whole_copies_do_not_retain_overwritten_histories() {
        let (mut a, mut b) = (solid(1), solid(2));
        for _ in 0..1000 {
            a = a.with_composite_layer(copy(b.clone()));
            b = b.with_composite_layer(copy(a.clone()));
            assert!(matches!(
                a,
                ImageInput::Solid {
                    color: [2, 2, 2, 2],
                    ..
                }
            ));
        }
    }
    #[test]
    fn partial_and_channel_only_copies_keep_destination_pixels() {
        for change in 0..5 {
            let mut layer = copy(solid(2));
            match change {
                0 => layer.math = ImageCompositeMath::CopyRgb,
                1 => layer.math = ImageCompositeMath::CopyAlpha,
                2 => layer.opacity = 254,
                3 => layer.clip = Some([0, 0, 1, 2]),
                _ => layer.source = [1, 0, 2, 2],
            }
            let result = solid(1).with_composite_layer(layer);
            let ImageInput::Composite { base, .. } = result else {
                panic!("lost destination")
            };
            assert!(matches!(
                *base,
                ImageInput::Solid {
                    color: [1, 1, 1, 1],
                    ..
                }
            ));
        }
    }
    #[test]
    fn full_atlas_copy_keeps_source_mapping_and_drops_old_base() {
        let mut layer = copy(ImageInput::Solid {
            size: [8, 8],
            color: [2; 4],
        });
        layer.source = [3, 4, 2, 2];
        let ImageInput::Composite { base, layers } = solid(1).with_composite_layer(layer) else {
            panic!()
        };
        assert!(matches!(
            *base,
            ImageInput::Solid {
                color: [0, 0, 0, 0],
                ..
            }
        ));
        assert_eq!(layers[0].source, [3, 4, 2, 2]);
    }
}
