# roder-app-server-core

The lightweight client/transcript layer shared by `roder-app-server` and its
consumers. Holds the `AppClient` trait, its event/notification receivers, and
the transcript recorder, with no dependency on the heavy `AppServer`
implementation so consumers can type-check against the trait surface in parallel
with the server crate.
