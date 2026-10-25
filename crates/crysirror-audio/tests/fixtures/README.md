# Synthetic audio fixtures

These files contain a self-authored 1 kHz sine, 48 kHz stereo, 0.75 seconds.
No game music, recording or licensed sample is included. Generated with local
ffmpeg 9.0.1; the tests consume the fixtures without requiring ffmpeg or a device.

```sh
ffmpeg -f lavfi -i sine=frequency=1000:sample_rate=48000:duration=0.75 \
  -ac 2 -c:a flac tone.flac
ffmpeg -f lavfi -i sine=frequency=1000:sample_rate=48000:duration=0.75 \
  -ac 2 -c:a libmp3lame -q:a 4 tone.mp3
ffmpeg -f lavfi -i sine=frequency=1000:sample_rate=48000:duration=0.75 \
  -ac 2 -c:a vorbis -strict -2 tone.ogg
```

The authored fixture content is released under the repository's MPL-2.0 license.
