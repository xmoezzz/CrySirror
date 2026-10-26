// SPDX-License-Identifier: GPL-3.0-or-later
//! S25/WARC native layout fixtures and explicitly enabled source-corpus tests.
use crysirror_core::{service::{AssetReader,MemoryAssetReader},state::ClientId};
use crysirror_resource::{DecodeContext,FormatRegistry,read_all};
use std::{sync::Arc,io::{Read,Write}};
fn memory(v:Vec<u8>)->Arc<dyn AssetReader>{Arc::new(MemoryAssetReader(Arc::from(v)))}
fn registry()->FormatRegistry{let mut r=FormatRegistry::default();crysirror_formats_arc::register(&mut r).unwrap();r}
fn put(v:&mut[u8],at:usize,n:u32){v[at..at+4].copy_from_slice(&n.to_le_bytes());}
#[test]
fn s25_shared_rows_apply_incremental_predictors_and_emit_real_png(){
    let mut data=vec![0;101];data[..4].copy_from_slice(b"S25\0");put(&mut data,4,2);put(&mut data,8,16);put(&mut data,12,40);
    for at in [16,40]{put(&mut data,at,3);put(&mut data,at+4,1);put(&mut data,at+16,0x80000000);put(&mut data,at+20,88);}
    data[88..90].copy_from_slice(&11u16.to_le_bytes());data[90..92].copy_from_slice(&0x4003u16.to_le_bytes());data[92..].copy_from_slice(&[10,20,30,1,2,3,2,3,4]);
    let cx=DecodeContext::new(ClientId(1));let (_,archive)=registry().open(memory(data),Some("shiina-rio/s25"),&cx).unwrap();assert_eq!(archive.entries().len(),2);
    for e in archive.entries(){let reader=archive.open(e.id,&cx).unwrap();let bytes=read_all(reader.as_ref(),1<<20).unwrap();let pixels=crysirror_image::decode_standard(&bytes).unwrap();assert_eq!([pixels.width,pixels.height],[3,1]);assert_eq!(pixels.rgba,[30,20,10,255,63,42,21,255,100,67,34,255]);assert_eq!(bytes.len()as u64,e.size);}
    assert_eq!(cx.resident_bytes(),0);
}
fn compressed(data:&[u8])->Vec<u8>{let mut z=flate2::write::ZlibEncoder::new(vec![],flate2::Compression::fast());z.write_all(data).unwrap();z.finish().unwrap()}
fn warc_record(out:&mut Vec<u8>,name:&str,offset:u32,size:u32,original:u32,flags:u32){let mut field=[0;16];field[..name.len()].copy_from_slice(name.as_bytes());out.extend_from_slice(&field);for word in [offset,size,original,0,0,flags]{out.extend_from_slice(&word.to_le_bytes());}}
fn warc_file(marker:&[u8;3],size:u32,body:&[u8],encrypted:bool)->Vec<u8>{let mut w=(u32::from(marker[0])|(u32::from(marker[1])<<8)|(u32::from(marker[2])<<16))^0x82ad82^size;if encrypted{w|=1<<24;}let mut out=w.to_le_bytes().to_vec();out.extend_from_slice(&size.to_le_bytes());out.extend_from_slice(body);out}
#[test]
fn warc_native_payloads_and_explicit_client_index_pipeline(){
    let raw=warc_file(b"RAW",3,b"abc",false);let z=warc_file(b"YPK",9,&compressed(b"resource!"),false);let huffman=warc_file(b"YH1",4,&((u32::from(b'Z'))<<23).to_le_bytes(),false);
    let offset=12+raw.len()+z.len()+huffman.len();let mut data=b"WARC 1.7".to_vec();data.extend_from_slice(&((offset as u32)^0xf182ad82).to_le_bytes());data.extend_from_slice(&raw);data.extend_from_slice(&z);data.extend_from_slice(&huffman);
    warc_record(&mut data,"raw.bin",12,raw.len()as u32,999,0);warc_record(&mut data,"pack.bin",12+raw.len()as u32,z.len()as u32,9,0);warc_record(&mut data,"huff.bin",12+raw.len()as u32+z.len()as u32,huffman.len()as u32,4,0);
    let mut cx=DecodeContext::new(ClientId(3));assert!(registry().open(memory(data.clone()),Some("shiina-rio/warc"),&cx).is_err());
    cx.parameters.insert("shiina-rio/warc".into(),serde_json::json!({"entry_name_size":16,"table_transforms":[{"algorithm":"identity"}]}));
    let (_,archive)=registry().open(memory(data),Some("shiina-rio/warc"),&cx).unwrap();assert_eq!(archive.entries().len(),3);
    for (e,expected) in archive.entries().iter().zip([b"abc".as_slice(),b"resource!",b"ZZZZ"]){let reader=archive.open(e.id,&cx).unwrap();assert_eq!(read_all(reader.as_ref(),1<<20).unwrap(),expected);}
    assert_eq!(archive.entries()[0].metadata["index_size"],999);assert_eq!(cx.resident_bytes(),0);
}
#[test]
fn invalid_s25_and_warc_inputs_fail_without_panics_or_budget_leaks(){
    let r=registry();let cx=DecodeContext::new(ClientId(4));for id in ["shiina-rio/s25","shiina-rio/warc"]{for n in 0..80 {let input=vec![0;n];assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(||r.open(memory(input),Some(id),&cx))).is_ok());assert_eq!(cx.resident_bytes(),0);}}
}
#[test]
#[ignore="requires complete pinned arc_unpacker checkout; no external image fixture is distributed"]
fn pinned_s25_corpus_pixels_match_original_expected_images(){
    let root=std::path::PathBuf::from(std::env::var("ARC_UNPACKER_SOURCE").expect("ARC_UNPACKER_SOURCE"));let head=std::process::Command::new("git").arg("-C").arg(&root).args(["rev-parse","HEAD"]).output().unwrap();assert_eq!(String::from_utf8(head.stdout).unwrap().trim(),"456834ecf2e5686813802c37efd829310485c57d");let dir=root.join("tests/dec/shiina_rio/files/s25");
    let r=registry();let cx=DecodeContext::new(ClientId(7));
    for (input,expected) in [("BLACK-zlib.S25",vec!["BLACK-out.png"]),("BLACK2-zlib.S25",vec!["BLACK-out.png"]),("CURTAIN1-zlib.S25",vec!["CURTAIN1-out.png"]),("ERRORF-zlib.S25",vec!["ERRORF-out.png"]),("DISPDATE-zlib.S25",vec!["DISPDATE_000-out.png","DISPDATE_001-out.png"]) ]{
        let packed=std::fs::read(dir.join(input)).unwrap();let mut bytes=vec![];flate2::read::ZlibDecoder::new(packed.as_slice()).read_to_end(&mut bytes).unwrap();let (_,archive)=r.open(memory(bytes),Some("shiina-rio/s25"),&cx).unwrap();assert_eq!(archive.entries().len(),expected.len(),"{input}");
        for (entry,name) in archive.entries().iter().zip(expected){let reader=archive.open(entry.id,&cx).unwrap();let decoded=crysirror_image::decode_standard(&read_all(reader.as_ref(),1<<26).unwrap()).unwrap();let expected=crysirror_image::decode_standard(&std::fs::read(dir.join(name)).unwrap()).unwrap();assert_eq!([decoded.width,decoded.height],[expected.width,expected.height]);for (pixel,want) in decoded.rgba.chunks_exact(4).zip(expected.rgba.chunks_exact(4)){assert!(pixel==want || (pixel[3]==0&&want[3]==0),"{input}/{name}: visible pixel {pixel:?} != {want:?}");}}
    }
    assert_eq!(cx.resident_bytes(),0);
}
