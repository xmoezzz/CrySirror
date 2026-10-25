// SPDX-License-Identifier: MPL-2.0
//! Borrowed native demux API shared by the BGI and Siglus frontends.
use na_mpeg2_decoder::{packet_ranges, StreamType};

#[test]
fn packet_ranges_borrow_exact_video_and_audio_payloads_and_reject_truncation() {
    fn pes(id: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0, 0, 1, id];
        out.extend(((payload.len() + 3) as u16).to_be_bytes());
        out.extend([0x80, 0, 0]);
        out.extend(payload);
        out
    }
    let mut stream = vec![0, 0, 1, 0xba, 0x44, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let video = pes(0xe0, &[1, 2, 3, 4]);
    let audio = pes(0xc0, &[5, 6, 7]);
    stream.extend(&video);
    stream.extend(&audio);
    let packets = packet_ranges(&stream);
    assert_eq!(packets.len(), 2);
    assert_eq!(packets[0].stream_type, StreamType::MpegVideo);
    assert_eq!(packets[1].stream_type, StreamType::MpegAudio);
    assert_eq!(&stream[packets[0].range.clone()], &[1, 2, 3, 4]);
    assert_eq!(&stream[packets[1].range.clone()], &[5, 6, 7]);
    stream.pop();
    assert_eq!(packet_ranges(&stream).len(), 1);
    assert!(packet_ranges(&[]).is_empty());
}

#[test]
fn raw_elementary_stream_is_borrowed_without_reinterpretation() {
    let bytes = [0, 0, 1, 0xb3, 0, 0, 0, 0];
    let packets = packet_ranges(&bytes);
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].range, 0..bytes.len());
    assert_eq!(packets[0].stream_type, StreamType::MpegVideo);
}
