use crysirror_video::theora;
use std::io::Cursor;

#[test]
fn vorbis_uses_reader_clock_and_omv_prefix_without_a_vm() {
    let ogg=include_bytes!("../../crysirror-audio/tests/fixtures/tone.ogg");
    let mut expected=Vec::new();
    theora::decode_audio(Cursor::new(ogg),||false,|c|{expected.extend(c.samples);Ok(true)}).unwrap();
    assert!(!expected.is_empty());
    assert!(expected.iter().any(|x|x.abs()>0.01));
    let mut omv=vec![0;0x58];
    omv[..4].copy_from_slice(&0x58u32.to_le_bytes());
    omv[0x28..0x2c].copy_from_slice(&2u32.to_le_bytes());
    omv[0x2c..0x30].copy_from_slice(&64u32.to_le_bytes());
    omv[0x30..0x34].copy_from_slice(&48u32.to_le_bytes());
    omv.extend_from_slice(b"index-padding");omv.extend_from_slice(ogg);
    let mut cursor=Cursor::new(omv);
    assert!(theora::is_input(&mut cursor).unwrap());assert_eq!(cursor.position(),0);
    let mut actual=Vec::new();let mut last=None;
    theora::decode_audio(cursor,||false,|c|{
        if let Some(pts)=last {assert!(c.pts_ms>=pts);}
        last=Some(c.pts_ms);actual.extend(c.samples);Ok(true)
    }).unwrap();
    assert_eq!(actual,expected);
    assert!(theora::decode_video(Cursor::new(b"broken"),||false,|_|Ok(true)).is_err());
    assert!(theora::decode_video(Cursor::new(b"broken"),||true,|_|panic!("cancelled decode")).unwrap().cancelled);
}

#[test]
#[ignore="requires owner VIDEO_FILE; never ships a game movie"]
fn owner_omv_uses_streamed_theora_planes_and_native_display_metadata() {
    let file=std::fs::File::open(std::env::var("VIDEO_FILE").expect("VIDEO_FILE")).unwrap();
    let mut count=0;
    let header=std::cell::Cell::new(None);
    theora::decode_video_with_metadata(file,||false,|info| {
        assert_eq!(info.duration_ms,None,"packet/page hints are not frame counts");
        assert!(info.frame_ms.is_some_and(|ms|ms>0.));
        header.set(Some(info));Ok(())
    },|frame|{
        assert_eq!(header.get().unwrap().size,[frame.width,frame.height]);
        assert_eq!(frame.rgba.len(),frame.width as usize*frame.height as usize*4);
        count+=1;Ok(count<120)
    }).unwrap();
    assert!(count>0);
}
