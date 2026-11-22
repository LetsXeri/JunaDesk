test-vp9.webm was generated from the FFmpeg testsrc2 filter with the command in the media test. It contains no third-party footage and is covered by the repository license.

test-h264.mp4 was generated with `ffmpeg -f lavfi -i testsrc2=size=96x64:rate=24:duration=1 -an -c:v libx264 -pix_fmt yuv420p test-h264.mp4`. It also contains only synthetic test footage.
