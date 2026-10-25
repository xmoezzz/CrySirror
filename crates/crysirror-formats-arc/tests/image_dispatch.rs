// SPDX-License-Identifier: GPL-3.0-or-later
use crysirror_image::DecoderRegistry;
#[test]
fn identical_raw_mask_layouts_use_declared_extensions_and_unknown_names_need_a_choice(){
 let mut r=DecoderRegistry::default();crysirror_formats_arc::register_images(&mut r).unwrap();
 let mut raw=2u32.to_le_bytes().to_vec();raw.extend(1u32.to_le_bytes());raw.extend([20,70]);
 assert_eq!(r.decode("pixels.fil",&raw).unwrap().rgba,[20,20,20,255,70,70,70,255]);
 assert_eq!(r.decode("pixels.mask",&raw).unwrap().rgba,[20,20,20,255,70,70,70,255]);
 assert!(r.decode("unknown.bin",&raw).is_err());
}
#[test]
fn strong_png_contents_outrank_a_weak_filename_based_decoder(){
 let mut r=DecoderRegistry::default();crysirror_formats_arc::register_images(&mut r).unwrap();
 let expected=crysirror_image::ImageSurface{width:1,height:1,rgba:vec![11,22,33,44]};let png=crysirror_image::encode_png(&expected).unwrap();
 assert_eq!(r.decode("actually-png.gr",&png).unwrap(),expected);
}
