// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
use super::*;
impl LayerSpec {
    /// Visit immutable image paths without copying glyph/mesh/vertex payloads.
    pub fn visit_image_dependencies(&self, visitor: &mut dyn FnMut(&str)) {
        if let Some(filter) = &self.filter {
            if let Some(tone) = &filter.tone_curve {
                visitor(&tone.asset);
            }
            if let Some(mask) = &filter.mask {
                visitor(&mask.asset);
            }
        }
        match &self.paint {
            Paint::Image { asset, .. } | Paint::Puppet { asset, .. } => visitor(asset),
            Paint::Snapshot { layers, .. } => {
                for layer in layers.iter() {
                    layer.visit_image_dependencies(visitor);
                }
            }
            Paint::Mesh { frame, .. } => {
                for asset in &frame.textures {
                    visitor(asset);
                }
            }
            Paint::Solid { .. } | Paint::Text { .. } | Paint::GlyphRun { .. } => (),
        }
    }
}
impl LayerSystem {
    /// Enumerate retained dependencies. False is a retirement barrier: an open
    /// transaction may still restore older paths that its journal retains.
    pub fn visit_image_dependencies(&self, visitor: &mut dyn FnMut(ClientId, &str)) -> bool {
        for (client, layers) in &self.clients {
            for layer in layers.values() {
                layer.visit_image_dependencies(&mut |asset| visitor(ClientId(*client), asset));
            }
        }
        for (client, transitions) in &self.crossfades {
            for transition in transitions.values() {
                if let Some(rule) = &transition.rule {
                    visitor(ClientId(*client), &rule.asset);
                }
                if let Some(SubtreeTransitionEffect::Dim { rule, .. }) = &transition.effect {
                    visitor(ClientId(*client), &rule.asset);
                }
            }
        }
        for ((client, _), frame) in &self.puppet_frames {
            for asset in &frame.textures {
                visitor(ClientId(*client), asset);
            }
        }
        if let Some(transition) = &self.transition {
            match &transition.spec.effect {
                TransitionEffect::Mask { asset, .. } => visitor(ClientId(transition.owner), asset),
                TransitionEffect::Siglus {
                    mask: Some(asset), ..
                } => visitor(ClientId(transition.owner), asset),
                _ => (),
            }
        }
        self.txn.0.is_empty()
    }
}
