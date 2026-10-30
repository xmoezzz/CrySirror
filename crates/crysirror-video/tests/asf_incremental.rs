use crysirror_video::asf;
use std::io::Cursor;
fn fixture()->Vec<u8> {include_bytes!("fixtures/wmv_wma.asf").to_vec()}

#[test]
fn selected_header_metadata_precedes_every_picture_and_can_reject_preparation() {
    let header = std::cell::Cell::new(None);
    let mut count = 0;
    asf::decode_video_with_metadata(Cursor::new(fixture()), || false,
        |info| { assert_eq!(info.size, [64,48]); assert!(info.duration_ms.is_some()); header.set(Some(info)); Ok(()) },
        |frame| { assert_eq!(header.get().unwrap().size, [frame.width,frame.height]); count += 1; Ok(true) }
    ).unwrap();
    assert_eq!(count, 4);
    let error = asf::decode_video_with_metadata(Cursor::new(fixture()), || false,
        |_| anyhow::bail!("header consumer rejected"),
        |_| panic!("rejected metadata must not publish a picture")
    ).unwrap_err();
    assert!(error.to_string().contains("header consumer rejected"));
}

#[test]
fn reader_video_and_float_audio_preserve_pts_and_decode_without_a_vm() {
    let mut reader=Cursor::new(fixture());
    assert!(asf::is_asf(&mut reader).unwrap());
    assert_eq!(reader.position(),0);
    let mut pictures=Vec::new();
    let stats=asf::decode_video(reader,||false,|f| {pictures.push(f);Ok(true)}).unwrap();
    assert_eq!(stats.outputs,4);
    assert!(pictures.windows(2).all(|p|p[1].pts_ms>p[0].pts_ms));
    for f in pictures {
        assert_eq!([f.width,f.height],[64,48]);
        assert!(f.rgba.chunks_exact(4).all(|p|p[0]>240 && p[1]<10 && p[2]<10 && p[3]==255));
    }
    let mut count=0; let mut peak=0f32; let mut last=None;
    asf::decode_audio(Cursor::new(fixture()),||false,|c|{
        assert_eq!((c.sample_rate,c.channels),(44100,1));
        if let Some(pts)=last {assert!(c.pts_ms>=pts);}
        last=Some(c.pts_ms);count+=c.samples.len();
        peak=c.samples.iter().fold(peak,|p,x|p.max(x.abs()));
        Ok(true)
    }).unwrap();
    assert!(count>4096);assert!(peak>0.01 && peak<1.0,"{peak}");
}

#[test]
fn invalid_headers_cancellation_and_callback_failure_are_distinct() {
    assert!(asf::decode_video(Cursor::new(vec![0;64]),||false,|_|Ok(true)).is_err());
    let cancelled=asf::decode_video(Cursor::new(fixture()),||true,|_|panic!("cancelled input decoded")).unwrap();
    assert!(cancelled.cancelled);assert_eq!(cancelled.outputs,0);
    let error=asf::decode_video(Cursor::new(fixture()),||false,|_|anyhow::bail!("consumer failed")).unwrap_err();
    assert!(error.to_string().contains("consumer failed"));
    let mut bytes=fixture();bytes[16..24].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(asf::decode_video(Cursor::new(bytes),||false,|_|Ok(true)).unwrap_err().to_string().contains("ASF metadata"));
}

#[test]
#[ignore="requires owner VIDEO_FILE; never ships a game movie"]
fn owner_movie_prefix_uses_the_same_reader_decoder() {
    let file=std::fs::File::open(std::env::var("VIDEO_FILE").expect("VIDEO_FILE")).unwrap();
    let mut count=0;
    asf::decode_video(file,||false,|f| {assert!(!f.rgba.is_empty());count+=1;Ok(count<120)}).unwrap();
    assert!(count>0);
}
