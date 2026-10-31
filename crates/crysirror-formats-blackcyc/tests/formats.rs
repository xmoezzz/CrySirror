use crysirror_core::{service::{AssetReader,MemoryAssetReader},state::ClientId};
use crysirror_resource::{ArchiveFormat,DecodeContext,FileFormat,read_all};
use crysirror_image::DecoderRegistry;
use std::sync::Arc;
fn reader(d:Vec<u8>)->Arc<dyn AssetReader>{Arc::new(MemoryAssetReader(d.into()))}
fn put32(d:&mut[u8],at:usize,v:u32){d[at..at+4].copy_from_slice(&v.to_le_bytes());}
fn put16(d:&mut[u8],at:usize,v:u16){d[at..at+2].copy_from_slice(&v.to_le_bytes());}
fn wrap(pack:&str,w:u32,h:u32,color:&[u8],mask:&[u8])->Vec<u8>{
    let mut out=vec![b' ';64];out[..16].fill(b' ');out[..4].copy_from_slice(if pack.starts_with('3'){b"PACK"}else{b"BMP "});
    put32(&mut out,32,color.len() as u32);put32(&mut out,36,w);put32(&mut out,40,h);
    let tag=format!("PACKTYPE={pack}");out[48..48+tag.len()].copy_from_slice(tag.as_bytes());out.extend(color);out.extend(mask);out
}
fn bmp(w:u32,h:i32,bpp:u16,colors:&[[u8;3]],data:&[u8])->Vec<u8>{
    let start=54+colors.len()*4;let mut d=vec![0;start];d[..2].copy_from_slice(b"BM");put32(&mut d,10,start as u32);put32(&mut d,14,40);put32(&mut d,18,w);put32(&mut d,22,h as u32);put16(&mut d,26,1);put16(&mut d,28,bpp);put32(&mut d,46,colors.len() as u32);
    for (i,c) in colors.iter().enumerate(){d[54+i*4..57+i*4].copy_from_slice(&[c[2],c[1],c[0]]);}
    d.extend(data);d
}
fn registry()->DecoderRegistry{let mut r=DecoderRegistry::default();crysirror_formats_blackcyc::register_images(&mut r).unwrap();r}
#[test]
fn dwq_plain_rgb_and_packed_bgr_are_top_down_with_palette_alpha(){
    let plain=bmp(2,2,24,&[],&[250,10,20,30,240,40,0,0,50,60,220,70,80,90,0,0]);
    let decoded=registry().decode("plain.dwq",&wrap("0",2,2,&plain,&[])).unwrap();
    assert_eq!(decoded.rgba,[250,10,20,255,30,240,40,255,50,60,220,255,70,80,90,255]);
    let mask=bmp(2,-2,8,&[[0,0,0],[192,192,192]],&[0,1,1,1,1]);
    let decoded=registry().decode("mask.dwq",&wrap("2A",2,2,&plain,&mask)).unwrap();
    assert_eq!(decoded.rgba,[250,10,20,0,30,240,40,192,50,60,220,192,70,80,90,0]);
    // Packed plane is BGR, with each row XORed against the preceding row.
    let bgr=[20,10,250,40,240,30,220,60,50,90,80,70];
    let delta:Vec<_>=bgr[..6].iter().copied().chain((0..6).map(|i|bgr[i+6]^bgr[i])).collect();
    assert!(!delta.contains(&0));
    let packed=bmp(2,-2,24,&[],&delta);
    let decoded=registry().decode("packed.dwq",&wrap("3A",2,2,&packed,&mask)).unwrap();
    assert_eq!(decoded.rgba,[250,10,20,0,30,240,40,192,50,60,220,192,70,80,90,0]);
    let overflow=bmp(2,2,8,&[[0,0,0],[255,255,255]],&[0,3]);
    assert!(registry().decode("overflow.dwq",&wrap("3A",2,2,&packed,&overflow)).is_err());
    let truncated=bmp(2,2,8,&[[0,0,0],[255,255,255]],&[0]);
    assert!(registry().decode("truncated.dwq",&wrap("2A",2,2,&plain,&truncated)).is_err());
    assert!(registry().decode("missing.dwq",&wrap("2A",2,2,&plain,&[])).is_err());
}
fn index_context(name:&str,index:Vec<u8>)->DecodeContext{
    let mut cx=DecodeContext::new(ClientId(1));cx.source_name=Some(name.into());
    let expected=if name.ends_with("gpk"){"data/sys.gtb"}else{"data/sys.vtb"}.to_owned();
    cx.resolver=Some(Arc::new(move|path:&str|->anyhow::Result<Arc<dyn AssetReader>>{anyhow::ensure!(path==expected,"unexpected scoped path {path}");Ok(reader(index.clone()))}));cx
}
#[test]
fn companion_archives_use_checked_monotone_ranges_and_no_body_copy(){
    let mut gtb=vec![0;12];put32(&mut gtb,0,1);gtb.extend(b"a\0");
    let cx=index_context("data/sys.gpk",gtb.clone());let archive=crysirror_formats_blackcyc::Gpk.open(reader(vec![1,2,3]),&cx).unwrap();
    assert_eq!(archive.entries()[0].path,"a.dwq");assert_eq!(read_all(archive.open(0,&cx).unwrap().as_ref(),10).unwrap(),[1,2,3]);
    put32(&mut gtb,8,4);assert!(crysirror_formats_blackcyc::Gpk.open(reader(vec![1,2,3]),&index_context("data/sys.gpk",gtb)).is_err());
    let mut vtb=vec![0;36];vtb[0]=b'A';vtb[12]=b'B';put32(&mut vtb,20,3);put32(&mut vtb,32,5);
    let cx=index_context("data/sys.vpk",vtb.clone());let archive=crysirror_formats_blackcyc::Vpk.open(reader(vec![1,2,3,4,5]),&cx).unwrap();
    assert_eq!(archive.entries()[1].path,"B.vaw");assert_eq!(read_all(archive.open(1,&cx).unwrap().as_ref(),10).unwrap(),[4,5]);
    put32(&mut vtb,32,2);assert!(crysirror_formats_blackcyc::Vpk.open(reader(vec![0;5]),&index_context("data/sys.vpk",vtb)).is_err());
    struct Sparse;impl AssetReader for Sparse {fn len(&self)->crysirror_core::Result<u64>{Ok(1<<30)}fn read_at(&self,_:u64,out:&mut[u8])->crysirror_core::Result<usize>{out.fill(7);Ok(out.len())}}
    let mut index=vec![0;12];put32(&mut index,0,1);put32(&mut index,8,(1<<30)-4);index.extend(b"sparse\0");
    let cx=index_context("data/sys.gpk",index);let archive=crysirror_formats_blackcyc::Gpk.open(Arc::new(Sparse),&cx).unwrap();
    assert_eq!(read_all(archive.open(0,&cx).unwrap().as_ref(),4).unwrap(),[7;4]);
}
#[test]
fn vaw_standard_payloads_and_bit_delta_preserve_actual_wave_bytes(){
    let wav=crysirror_resource::WaveSpec{format:1,channels:1,sample_rate:8000,bits_per_sample:16,..Default::default()}.encode(&[]).unwrap();
    let prefix=20+u32::from_le_bytes(wav[16..20].try_into().unwrap()) as usize;
    assert_eq!(wav.len(),prefix+8);
    let cx=DecodeContext::new(ClientId(1));let source=reader(wrap("0",0,0,&wav,&[]));
    let f=crysirror_formats_blackcyc::Vaw.decode("test.vaw",source,&cx).unwrap();
    assert_eq!(f.extension,"wav");assert_eq!(read_all(f.reader.as_ref(),100).unwrap(),wav);
    // A complete authored RIFF data header, encoded with native 4-bit widths.
    let mut bitstream=Vec::new();let mut at=0;
    for (value,n) in [(15u32,4usize),(8548,15),(5,4),(0,5),(15,4),(((-24949i32)^0x4000) as u32&0x7fff,15),(0,4)] {
        for bit in 0..n {if at%8==0{bitstream.push(0);}bitstream[at/8]|=(((value>>bit)&1)as u8)<<(at%8);at+=1;}
    }
    let mut compressed=wav[..prefix].to_vec();compressed.extend(&bitstream);
    let file=wrap("1",0,0,&compressed,&[]);let out=crysirror_formats_blackcyc::Vaw.decode("delta.vaw",reader(file.clone()),&cx).unwrap();
    assert_eq!(read_all(out.reader.as_ref(),100).unwrap(),wav);
    let mut bad=file;bad.truncate(64+prefix+1);assert!(crysirror_formats_blackcyc::Vaw.decode("bad.vaw",reader(bad),&cx).is_err());
}

#[test]
#[ignore="reads owner archives through scoped read-at readers; no game data distributed"]
fn owner_gpk_dwq_vpk_resources(){
    use std::{path::PathBuf,sync::Mutex,io::{Read,Seek,SeekFrom},collections::{BTreeMap,BTreeSet}};
    struct File(Mutex<std::fs::File>,u64);impl AssetReader for File {
        fn len(&self)->crysirror_core::Result<u64>{Ok(self.1)}
        fn read_at(&self,at:u64,out:&mut[u8])->crysirror_core::Result<usize>{let mut f=self.0.lock().unwrap();f.seek(SeekFrom::Start(at)).and_then(|_|f.read(out)).map_err(|e|crysirror_core::Error::Io(e.to_string()))}
    }
    fn open(p:PathBuf)->Arc<dyn AssetReader>{let f=std::fs::File::open(p).unwrap();let len=f.metadata().unwrap().len();Arc::new(File(Mutex::new(f),len))}
    let root=PathBuf::from(std::env::var_os("CRYSIRROR_TEST_BLACKCYC_ROOT").unwrap());let images=registry();let mut counts=BTreeMap::<String,usize>::new();let mut captures=BTreeSet::new();let mut audio=0;let mut pcm_frames=0;let mut peak=0f32;
    for dir in ["dwq","cdvaw"] {
        let mut files:Vec<_>=std::fs::read_dir(root.join(dir)).unwrap().map(|v|v.unwrap().path()).filter(|p|p.extension().is_some_and(|e|e==if dir=="dwq"{"gpk"}else{"vpk"})).collect();files.sort();
        for file in files {
            let mut cx=DecodeContext::new(ClientId(1));cx.source_name=Some(format!("{dir}/{}",file.file_name().unwrap().to_str().unwrap()));let capability=root.clone();
            cx.resolver=Some(Arc::new(move|name:&str|->anyhow::Result<Arc<dyn AssetReader>>{let p=crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset,name)?;Ok(open(capability.join(p.relative())))}));
            let archive=if dir=="dwq"{crysirror_formats_blackcyc::Gpk.open(open(file),&cx)}else{crysirror_formats_blackcyc::Vpk.open(open(file),&cx)}.unwrap();
            for entry in archive.entries() {
                let source=archive.open(entry.id,&cx).unwrap();
                if dir=="dwq" {
                    let bytes=read_all(source.as_ref(),64<<20).unwrap();let image=images.decode(&entry.path,&bytes).unwrap();image.validate().unwrap();
                    let pack=std::str::from_utf8(&bytes[48..64]).unwrap().trim().to_owned();*counts.entry(pack.clone()).or_default()+=1;
                    if captures.insert(pack.clone()) {if let Some(dir)=std::env::var_os("CRYSIRROR_TEST_BLACKCYC_CAPTURE_DIR") {
                        let output=PathBuf::from(dir);std::fs::create_dir_all(&output).unwrap();std::fs::write(output.join(format!("{}.png",pack.replace('=',"-"))),crysirror_image::encode_png_uncompressed(&image).unwrap()).unwrap();
                    }}
                } else if entry.id%97==0 {
                    let resource=crysirror_formats_blackcyc::Vaw.decode(&entry.path,source,&cx).unwrap();let mut mark=[0;4];resource.reader.read_at(0,&mut mark).unwrap();assert!(mark==*b"RIFF"||mark==*b"OggS");
                    let mut decoder=crysirror_audio::decoder::open_asset(resource.reader.clone(),Some(&resource.extension)).unwrap();
                    let mut frames=[[0f32;2];2048];let mut read=0;
                    for _ in 0..8 {let n=decoder.read(&mut frames).unwrap();read+=n;for c in frames[..n].iter().flatten(){assert!(c.is_finite());peak=peak.max(c.abs());}if n==0{break;}}
                    assert!(read>0,"owner audio must actually decode PCM");pcm_frames+=read;audio+=1;
                }
            }
        }
    }
    println!("owner_dwq_profiles={counts:?} sampled_audio_resources={audio} decoded_pcm_frames={pcm_frames} peak={peak}");assert_eq!(counts.values().sum::<usize>(),1625);assert!(audio>0&&peak>0.);
}
