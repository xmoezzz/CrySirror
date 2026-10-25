// SPDX-License-Identifier: MPL-2.0
use anyhow::{bail, Result};
use crysirror_image::{DecoderRegistry, ImageDecodeContext, ImageFormat, ImageSurface};
use std::sync::Arc;
struct Pixel {
    id: &'static str,
    priority: u8,
    color: u8,
}
impl ImageFormat for Pixel {
    fn id(&self) -> &'static str {
        self.id
    }
    fn priority(&self) -> u8 {
        self.priority
    }
    fn recognize(&self, _: &str, b: &[u8]) -> bool {
        b == b"pixel"
    }
    fn decode(&self, _: &[u8], _: &ImageDecodeContext<'_>) -> Result<ImageSurface> {
        Ok(ImageSurface {
            width: 1,
            height: 1,
            rgba: vec![self.color, 0, 0, 255],
        })
    }
}
#[test]
fn specific_format_wins_but_equal_specificity_requires_explicit_selection() {
    let mut r = DecoderRegistry::default();
    r.register_format(Arc::new(Pixel {
        id: "generic",
        priority: 10,
        color: 10,
    }))
    .unwrap();
    r.register_format(Arc::new(Pixel {
        id: "specific",
        priority: 50,
        color: 50,
    }))
    .unwrap();
    assert_eq!(r.decode("a.bin", b"pixel").unwrap().rgba, [50, 0, 0, 255]);
    r.register_format(Arc::new(Pixel {
        id: "equally-specific",
        priority: 50,
        color: 70,
    }))
    .unwrap();
    assert!(r.decode("a.bin", b"pixel").is_err());
    assert_eq!(
        r.decode_with("a.bin", b"pixel", None, None, Some("equally-specific"))
            .unwrap()
            .rgba,
        [70, 0, 0, 255]
    );
    assert!(r
        .decode_with("a.bin", b"bad", None, None, Some("specific"))
        .is_err());
}
struct Reference;
impl ImageFormat for Reference {
    fn id(&self) -> &'static str {
        "reference"
    }
    fn recognize(&self, _: &str, b: &[u8]) -> bool {
        b.starts_with(b"REF:")
    }
    fn decode(&self, b: &[u8], ctx: &ImageDecodeContext<'_>) -> Result<ImageSurface> {
        ctx.decode_relative(std::str::from_utf8(&b[4..])?)
    }
}
#[test]
fn recursive_images_and_path_escapes_fail_and_optional_dependencies_only_ignore_notfound() {
    let mut r = DecoderRegistry::default();
    r.register_format(Arc::new(Reference)).unwrap();
    let source = |path: &str| {
        assert_eq!(path, "pack/self.ref");
        Ok(b"REF:self.ref".to_vec())
    };
    assert!(r
        .decode_with("pack/self.ref", b"REF:self.ref", Some(&source), None, None)
        .is_err());
    let escape =
        |_: &str| -> crysirror_core::Result<Vec<u8>> { panic!("escape must not reach source") };
    assert!(r
        .decode_with(
            "pack/outer.ref",
            b"REF:../private",
            Some(&escape),
            None,
            None
        )
        .is_err());
    let missing = |_: &str| Err(crysirror_core::Error::NotFound);
    let ctx = ImageDecodeContext::new("pack/a.ref", &r).with_source(&missing);
    assert!(ctx.try_read_relative("optional.png").unwrap().is_none());
    let denied = |_: &str| Err(crysirror_core::Error::ReadOnly);
    let ctx = ImageDecodeContext::new("pack/a.ref", &r).with_source(&denied);
    assert!(ctx.try_read_relative("optional.png").is_err());
    let ctx = ImageDecodeContext::new("pack/a.ref", &r);
    assert!(ctx.try_read_relative("optional.png").is_err());
}
struct Invalid;
impl ImageFormat for Invalid {
    fn id(&self) -> &'static str {
        "invalid"
    }
    fn recognize(&self, _: &str, _: &[u8]) -> bool {
        true
    }
    fn decode(&self, b: &[u8], _: &ImageDecodeContext<'_>) -> Result<ImageSurface> {
        if b == b"error" {
            bail!("native corrupt stream")
        };
        Ok(ImageSurface {
            width: 2,
            height: 2,
            rgba: vec![0; 4],
        })
    }
}
#[test]
fn unregistered_ids_corrupt_streams_and_invalid_pixel_lengths_do_not_succeed() {
    let mut r = DecoderRegistry::default();
    r.register_format(Arc::new(Invalid)).unwrap();
    assert!(r.decode("a", b"error").is_err());
    assert!(r.decode("a", b"bad pixel length").is_err());
    assert!(r
        .decode_with("a", b"anything", None, None, Some("unknown"))
        .is_err());
}
