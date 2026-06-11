# Roder Speech (Speech-to-Text)

Roder exposes a provider-neutral speech-to-text service. Clients (TUI,
desktop, SDKs, automations) own microphone capture; Roder accepts bounded
audio payloads and normalizes provider results into stable speech
primitives (`text`, `language`, `durationMillis`, `segments` with optional
timestamps/speaker labels/confidence, and raw provider `metadata`).

## Providers

| Provider id | Models | Auth |
| --- | --- | --- |
| `openai-speech` | `gpt-4o-transcribe`, `gpt-4o-mini-transcribe`, `gpt-4o-transcribe-diarize`, `whisper-1`, `gpt-realtime-whisper` (streaming metadata only) | `OPENAI_API_KEY` (or `RODER_OPENAI_SPEECH_API_KEY`), optional `OPENAI_BASE_URL` |
| `google-speech` | `chirp_3`, `chirp`, `latest_long`, `latest_short`, `long`, `short` | OAuth: `RODER_GOOGLE_SPEECH_ACCESS_TOKEN` + `RODER_GOOGLE_SPEECH_PROJECT`/`GOOGLE_CLOUD_PROJECT`; or API key: `RODER_GOOGLE_SPEECH_API_KEY`/`GEMINI_API_KEY`/`GOOGLE_API_KEY`; or Application Default Credentials (authorized-user ADC JSON via `GOOGLE_APPLICATION_CREDENTIALS` or `~/.config/gcloud/`, else the gcloud CLI; `RODER_GCLOUD_BIN` overrides the binary). Optional `RODER_GOOGLE_SPEECH_LOCATION`, `RODER_GOOGLE_SPEECH_ENDPOINT` |

Both providers are installed by default, so `speech/providers/list` shows
them (with `authenticated: false`) even before credentials are configured.
`gpt-realtime-whisper` is advertised as streaming-capability metadata and is
rejected by `speech/transcribe` with an explanatory error.

## App-Server API

Discover providers and models:

```json
{"method": "speech/providers/list"}
```

Transcribe a bounded audio payload (base64; the client owns capture and
encoding):

```json
{
  "method": "speech/transcribe",
  "params": {
    "provider": "openai-speech",
    "model": "gpt-4o-mini-transcribe",
    "audio": {
      "bytesBase64": "<base64 wav/mp3/m4a/ogg bytes>",
      "mimeType": "audio/wav",
      "filename": "clip.wav"
    },
    "language": "en",
    "prompt": "optional biasing prompt",
    "diarization": false
  }
}
```

Omitting `provider`/`model` selects the first installed provider and its
first model. Invalid base64 fails with `-32602` before any provider call.
Speech synthesis uses the parallel `speech/synthesis/providers/list` and
`speech/synthesize` methods.

## CLI

```sh
roder speech providers
roder speech transcribe clip.wav --provider openai-speech --model gpt-4o-mini-transcribe --language en
cat clip.wav | roder speech transcribe - --format json
roder speech transcribe clip.wav --to-thread <thread-id>   # transcript becomes a turn prompt
```

Missing credentials produce explicit errors naming the env vars to set.
Transcription output is text by default; `--format json` prints the full
normalized result. Sending a transcript into a turn is an explicit follow-up
step (paste or scripting) — Roder never starts a turn from audio silently.

## Privacy And Redaction

- Roder does not store raw audio in thread transcripts or session stores;
  audio bytes only travel in the `speech/transcribe` request to the chosen
  provider.
- API transcript recording (`--record-api-transcript`) redacts raw audio
  payloads (`audio.bytesBase64`, `voiceSample.bytesBase64`) and sensitive
  keys (API keys, tokens, bearer headers) before writing JSONL, so captured
  transcripts stay shareable for debugging.
- Provider diagnostics embedded in results (`metadata`) come from the
  provider response body and never include your API key.

## Testing

Offline (default, no network or credentials):

```sh
cargo test -p roder-ext-openai-speech
cargo test -p roder-ext-google-speech
cargo test -p roder-app-server speech
```

These cover model listings, missing-auth errors, multipart/JSON request
body shapes against fake HTTP servers, response/error parsing, and the
JSON-RPC success path through an offline fake transcriber.

Opt-in live validation (synthesizes a small sine-tone WAV in-process; see
`tests/fixtures/audio/README.md`):

```sh
RODER_OPENAI_SPEECH_LIVE=1 OPENAI_API_KEY=... \
  cargo test -p roder-ext-openai-speech --test live_openai_speech -- --ignored
RODER_GOOGLE_SPEECH_LIVE=1 RODER_GOOGLE_SPEECH_ACCESS_TOKEN=... RODER_GOOGLE_SPEECH_PROJECT=... \
  cargo test -p roder-ext-google-speech --test live_google_speech -- --ignored
```

## Known Gaps

- Google ADC is supported for authorized-user credentials (refresh-token
  flow, cached with early expiry) and via the gcloud CLI fallback.
  Service-account key JSON is rejected with guidance — RS256 request
  signing would need a crypto dependency; use
  `gcloud auth application-default login` instead.
- Streaming transcription (`gpt-realtime-whisper`) is metadata-only; there is
  no public streaming session API yet.
- `roder speech transcribe <audio> --to-thread <thread-id>` forwards the
  finished transcript into `turn/start` on that thread. The flag is the
  explicit consent: turns are never started from audio without it.
