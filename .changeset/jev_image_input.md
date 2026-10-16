---
roder-ext-jev: patch
---

# Opt into TypeSafe image input for Jev decisions

`JEV_IMAGES=1` sends the current viewport screenshot with each hosted Jev decision, fitted to TypeSafe's limits (PNG or JPEG, 280 to 1568 px per side) and referenced from the request state, with the `TypeSafe-Danger-Zone-Images: true` header. Screenshots are suppressed around recognized secret fields, as for Decisions. Off by default; the text-only request is unchanged.
