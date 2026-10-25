// SPDX-License-Identifier: MPL-2.0
use crysirror_image::{DecoderRegistry, ImageDecoder};
use crysirror_siglus::{g00::decode_g00, SiglusImageDecoder};
use std::sync::Arc;

fn literals(raw: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for chunk in raw.chunks(8) {
        bytes.push(((1u16 << chunk.len()) - 1) as u8);
        bytes.extend(chunk);
    }
    bytes
}
fn compressed(raw: &[u8]) -> Vec<u8> {
    let payload = literals(raw);
    let mut bytes = Vec::new();
    // The native runtime decoder deliberately ignores arc_size.
    bytes.extend(0u32.to_le_bytes());
    bytes.extend((raw.len() as u32).to_le_bytes());
    bytes.extend(payload);
    bytes
}
fn type2() -> Vec<u8> {
    let mut cut = vec![0u8; 0x74];
    cut[2..4].copy_from_slice(&1u16.to_le_bytes());
    for (offset, value) in [
        (4, 10i32),
        (8, 20),
        (12, 2),
        (16, 1),
        (20, 11),
        (24, 20),
        (28, 32),
        (32, 32),
    ] {
        cut[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    let mut chip = vec![0u8; 0x5c];
    chip[0..2].copy_from_slice(&10u16.to_le_bytes());
    chip[2..4].copy_from_slice(&20u16.to_le_bytes());
    chip[6..8].copy_from_slice(&2u16.to_le_bytes());
    chip[8..10].copy_from_slice(&1u16.to_le_bytes());
    cut.extend(chip);
    cut.extend([1, 2, 3, 4, 5, 6, 7, 8]);
    let mut raw = 3u32.to_le_bytes().to_vec();
    for (offset, length) in [(28u32, cut.len() as i32), (28, -1), (0, 0)] {
        raw.extend(offset.to_le_bytes());
        raw.extend(length.to_le_bytes());
    }
    raw.extend(cut);
    let mut file = vec![2, 32, 0, 32, 0];
    file.extend(3u32.to_le_bytes());
    file.extend([0; 72]);
    file.extend(compressed(&raw));
    file
}
#[test]
fn native_type2_slots_crops_centers_and_reused_cuts_are_preserved() {
    let file = type2();
    let decoded = decode_g00(&file).unwrap();
    assert_eq!(decoded.original_sizes, [(32, 32), (32, 32), (0, 0)]);
    assert_eq!(decoded.frames.len(), 3);
    for frame in &decoded.frames[..2] {
        assert_eq!(
            (frame.width, frame.height, frame.center_x, frame.center_y),
            (2, 1, 1, 0)
        );
        assert_eq!(frame.rgba, [3, 2, 1, 4, 7, 6, 5, 8]);
    }
    assert_eq!(decoded.frames[2].rgba, [0; 4]);
    assert!(SiglusImageDecoder.decode(&file).is_err());
    for end in 0..file.len() {
        assert!(decode_g00(&file[..end]).is_err(), "truncation {end}");
    }
}
#[test]
fn literal_and_overlapping_type0_enters_shared_decoder_registry() {
    let mut file = vec![0, 3, 0, 1, 0];
    file.extend(0u32.to_le_bytes());
    file.extend(12u32.to_le_bytes());
    file.extend([1, 10, 20, 30, 0x11, 0]);
    let mut registry = DecoderRegistry::default();
    registry
        .register("g00", Arc::new(SiglusImageDecoder))
        .unwrap();
    let image = registry.decode("sprite.G00", &file).unwrap();
    assert_eq!(image.rgba, [30, 20, 10, 255].repeat(3));
    for end in 0..file.len() {
        assert!(decode_g00(&file[..end]).is_err());
    }
}
#[test]
fn malformed_and_excessive_lengths_fail_before_allocation() {
    let mut file = vec![0, 1, 0, 1, 0];
    file.extend(0u32.to_le_bytes());
    file.extend(u32::MAX.to_le_bytes());
    assert!(decode_g00(&file).is_err());
    file[0] = 255;
    assert!(decode_g00(&file).is_err());
    let mut file = type2();
    file[9 + 72 + 4..9 + 72 + 8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(decode_g00(&file).is_err());
}
