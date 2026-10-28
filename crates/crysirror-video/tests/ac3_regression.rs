//! Regression for the shared OxideAV core upgrade. Synthetic 440 Hz, 48 kHz,
//! stereo AC-3: no game input, decoder subprocess or audio device.
#[test]
fn ac3_tone_survives_core_dependency_alignment() {
    let mut decoder = na_mpeg2_decoder::Ac3AudioDecoder::new();
    let mut chunks = Vec::new();
    decoder
        .push_with(include_bytes!("fixtures/tone.ac3"), Some(0), |chunk| {
            chunks.push(chunk)
        })
        .unwrap();
    assert_eq!(chunks.len(), 5);
    for (i, chunk) in chunks.iter().enumerate() {
        assert_eq!((chunk.sample_rate, chunk.channels), (48_000, 2));
        assert_eq!(chunk.samples.len(), 1536 * 2);
        assert_eq!(chunk.pts_ms, i as i64 * 32);
        assert!(chunk.samples.iter().all(|x| x.is_finite()));
        let energy = chunk.samples.iter().map(|x| x * x).sum::<f32>() / chunk.samples.len() as f32;
        assert!(energy > 0.0001 && energy < 0.05, "tone energy {energy}");
    }
    // Positive-going crossings after the priming frame remain approximately
    // 440 Hz; catches silence/white-noise substitution, not just nonempty output.
    let left: Vec<_> = chunks
        .iter()
        .skip(1)
        .flat_map(|c| c.samples.chunks_exact(2).map(|s| s[0]))
        .collect();
    let crossings = left.windows(2).filter(|s| s[0] <= 0. && s[1] > 0.).count();
    assert!((53..=59).contains(&crossings), "tone crossings {crossings}");
}
