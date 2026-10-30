Synthetic red video and 440 Hz sine, authored with local FFmpeg solely for tests:

```
ffmpeg -f lavfi -i color=c=red:s=64x48:r=10:d=0.4 \
  -f lavfi -i sine=frequency=440:sample_rate=44100:duration=0.4 \
  -c:v wmv2 -c:a wmav2 wmv_wma.asf
```

No game content. Production decoding uses the Rust engine codec, with no decoder
process. The tests establish streaming, timestamps, pixels and non-silent WMA;
they do not establish all WMV/WMA profile conformance.
