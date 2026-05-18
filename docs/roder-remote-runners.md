# Roder Remote Runners

Remote runners let Roder execute workspace file operations and commands outside the local process while keeping the local filesystem as the default behavior.

## Mounts

Use mounts when the runner needs repeated access to a large dataset, cache, object-store prefix, or provider-native volume. Do not paste large storage contents into prompt context when the data should stay outside transcripts.

Roder models mounts as provider-neutral intents:

- `s3`
- `gcs`
- `r2`
- `azure_blob`
- `box_storage`
- `provider_native`

Mount credentials are references, not credential values. A mount should name a secret id such as `aws-prod-readonly`; raw tokens, API keys, or connection strings must not appear in runner config, prompts, transcripts, snapshots, or artifacts.

## Artifacts

Runner providers can export generated files or directories through `RunnerArtifactExportRequest`. The result contains a stable `artifact_id`, the exported path, and an optional provider URL. Providers that do not support artifact export return a clear unsupported error from the default runner API method.

## Snapshots

Snapshots capture runner state, not mounted remote storage. Snapshot metadata must not include mount paths, mount URIs, or secret-like keys and values. Providers should use snapshot metadata for deterministic restore handles only.

## Secrets

Hosted runner config supports credential validation at destination selection. Missing credentials fail before a runner session is selected. Error text should mention the expected environment variable, such as `BLAXEL_API_KEY`, without echoing secret values.

## Ports

Port previews use `RunnerPortRequest` and return a provider URL when supported. Local and Docker runners map this to localhost-style URLs; hosted providers should return their preview endpoint.
