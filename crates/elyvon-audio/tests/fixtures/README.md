# Audio fixtures

`tone.flac`, `tone.mp3` and `tone.ogg` are synthetic test signals created for this
repository (no third-party audio; CC0-1.0 / public domain dedication):

* 0.5 s, 44 100 Hz, stereo, 16-bit source
* left = 0.5 · sin(2π·440·t), right = 0.25 · sin(2π·660·t), samples `round(x · 32767)`

Regenerate (ffmpeg with libmp3lame and libvorbis):

```bash
python3 -c 'import math,struct,wave;r=44100;w=wave.open("src.wav","wb");w.setnchannels(2);w.setsampwidth(2);w.setframerate(r);w.writeframes(b"".join(struct.pack("<hh",int(round(0.5*math.sin(2*math.pi*440*i/r)*32767)),int(round(0.25*math.sin(2*math.pi*660*i/r)*32767))) for i in range(r//2)));w.close()'
ffmpeg -i src.wav -c:a flac tone.flac
ffmpeg -i src.wav -c:a libmp3lame -b:a 192k tone.mp3
ffmpeg -i src.wav -c:a libvorbis -q:a 6 tone.ogg
```

WAV (16/24-bit, float) and AIFF inputs are written by the tests themselves.
