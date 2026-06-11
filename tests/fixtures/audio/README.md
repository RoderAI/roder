# Audio Fixtures

Roder speech tests do not ship binary audio files. Every test that needs
audio synthesizes it in-process:

- Offline fake-HTTP tests use small literal byte strings because the fake
  server never decodes audio.
- Opt-in live tests (`live_openai_speech`, `live_google_speech`) build a
  valid 16-bit mono PCM WAV in memory: a 440 Hz sine tone, 0.4 seconds at
  8 kHz (~6.4 KB). See `synthetic_wav()` in
  `crates/roder-ext-openai-speech/tests/live_openai_speech.rs` and
  `crates/roder-ext-google-speech/tests/live_google_speech.rs`.

The generated tone contains no speech, no third-party content, and no
licensing constraints. Live checks therefore validate auth, request mapping,
upload, and response parsing — not exact transcript text.

If a future test needs a spoken-word fixture, generate it with a local TTS
tool, keep it under 100 KB, and document its creation command and license
here before committing it.
