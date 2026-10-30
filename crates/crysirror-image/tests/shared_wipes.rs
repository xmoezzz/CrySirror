use crysirror_image::{
    associated::{coverage_threshold_lut, map_coverage_alpha},
    wipe_mask::generate,
};

#[test]
fn extracted_wipes_compose_through_engine_coverage_without_a_vm() {
    let a = std::thread::spawn(|| generate(40, &[0, 4, 4], 48, 32, 123).unwrap())
        .join()
        .unwrap();
    let b = generate(40, &[0, 4, 4], 48, 32, 123).unwrap();
    let c = generate(40, &[0, 4, 4], 48, 32, 456).unwrap();
    assert_eq!(a.pixels, b.pixels);
    assert_ne!(a.pixels, c.pixels);
    let mut pixels = [23, 45, 67, 0].repeat(48 * 32);
    map_coverage_alpha(&mut pixels, &a.pixels, &coverage_threshold_lut(false, 510)).unwrap();
    assert!(pixels.chunks_exact(4).all(|p| p == [23, 45, 67, 255]));
    map_coverage_alpha(&mut pixels, &a.pixels, &coverage_threshold_lut(false, 0)).unwrap();
    assert!(pixels.chunks_exact(4).all(|p| p == [23, 45, 67, 0]));
    assert!(generate(40, &[], 0, 32, 0).is_none());
    assert!(generate(40, &[], 8192, 8192, 0).is_none());
    assert!(generate(-123, &[], 48, 32, 0).is_none());
}
