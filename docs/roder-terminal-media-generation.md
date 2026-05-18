# Roder Terminal Media Generation

Roder models generated images and videos as provider-neutral media artifacts. Providers can expose generation tools, while app-server clients and the TUI use one artifact contract for metadata, reads, previews, deletion, and later-turn attachments.

## Tools

- `media.generate_image`: create an image artifact.
- `media.generate_video`: create a video artifact.
- `media.describe`: inspect artifact metadata without reading bytes.
- `media.attach`: convert artifact bytes into an attachment payload.

Normal tests use deterministic fake media tools. Live providers must be opt in behind `RODER_MEDIA_LIVE=1` and provider-specific credentials.

## Storage

Artifacts live under `~/.roder/artifacts/` by default. Set `[media].artifacts_dir` or `RODER_MEDIA_ARTIFACT_DIR` to override the store for tests or a controlled profile.

The app-server refuses reads over the configured byte cap and only deletes Roder-owned artifacts.

## App-Server Methods

- `media/list`: return metadata without reading bytes.
- `media/read`: return capped base64 bytes plus metadata.
- `media/thumbnail`: return preview metadata.
- `media/delete`: delete Roder-owned metadata and bytes.
- `media/attachToTurn`: return a `MediaAttachment` and, for images, an `InputImage` data URL compatible with `turns/start` and `turns/steer`.

Events:

- `media/artifactCreated`
- `media/artifactUpdated`
- `media/artifactDeleted`
- `media/previewReady`

## TUI Behavior

The TUI has compact media rows that show provider, MIME type, byte size, artifact path, and preview fallback labels. If inline image protocols are unavailable, the preview degrades to metadata and a path. Media palette entries seed `/imagegen` and `/videogen` prompts.

Generated image attachments reuse the existing image attachment path, so providers that support image input receive base64 data URLs.

## Reference Frames

The roadmap reference frames are stored under `roadmap/assets/grok-build-2026-05-16/`, including `media-generation-contact-sheet.png` and `frames/media-generation-02.png`.
