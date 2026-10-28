"""Regenerate self-authored MP4 fixtures. FFmpeg is only a test-data tool.

The committed fixtures were generated with FFmpeg 9.0.1, libx264/libx265,
and SVT-AV1 4.2.0. Runtime and cargo tests never invoke this program.
"""
from pathlib import Path
import shutil
import subprocess

DEST = Path(__file__).resolve().parent
FFMPEG = shutil.which("ffmpeg")
CASES = {
    "h264": ["-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p", "-profile:v", "baseline", "-g", "3"],
    "h264_main": ["-c:v", "libx264", "-preset", "medium", "-pix_fmt", "yuv420p", "-profile:v", "main", "-bf", "2", "-g", "3"],
    "h265": ["-c:v", "libx265", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-x265-params", "log-level=error:pools=1:keyint=3"],
    "h265_10": ["-c:v", "libx265", "-preset", "ultrafast", "-pix_fmt", "yuv420p10le", "-x265-params", "log-level=error:pools=1:keyint=3"],
    "av1": ["-c:v", "libsvtav1", "-preset", "11", "-pix_fmt", "yuv420p", "-g", "3", "-svtav1-params", "lp=1"],
    "av1_10": ["-c:v", "libsvtav1", "-preset", "11", "-pix_fmt", "yuv420p10le", "-g", "3", "-svtav1-params", "lp=1"],
}


def run(*args, capture=False):
    return subprocess.run([FFMPEG, "-hide_banner", "-loglevel", "error", *map(str, args)],
                          check=True, stdout=subprocess.PIPE if capture else None).stdout


def main():
    if not FFMPEG:
        raise SystemExit("FFmpeg required to regenerate fixtures, never to run cargo tests")
    for name, options in CASES.items():
        movie = DEST / f"{name}.mp4"
        run("-f", "lavfi", "-i", "testsrc2=size=64x64:rate=5", "-t", "1", "-an", *options, "-y", movie)
        # The engine uses nearest-neighbour chroma expansion. Align the 10-bit
        # reference's resampler; otherwise swscale chooses a different filter.
        flags = ["-sws_flags", "neighbor+bitexact"] if name.endswith("_10") else []
        rgba = run("-i", movie, *flags, "-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1", capture=True)
        assert len(rgba) == 5 * 64 * 64 * 4
        samples = bytearray()
        for frame in range(5):
            for sample in range(64):
                offset = frame * 16384 + ((sample * 1297 + 113) % 4096) * 4
                samples += rgba[offset:offset + 3]
        (DEST / f"{name}.oracle.rgb").write_bytes(samples)
    run("-i", DEST / "h264.mp4", "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:duration=1",
        "-c:v", "copy", "-c:a", "aac", "-b:a", "32k", "-shortest", "-y", DEST / "h264_aac.mp4")
    run("-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:duration=0.16",
        "-c:a", "ac3", "-ac", "2", "-b:a", "192k", "-y", DEST / "tone.ac3")


if __name__ == "__main__":
    main()
