---
roder-extension-host: major
roder-app-server: patch
roder-dist-hosted: patch
roder: patch
---

# Let hosts choose Roder's data folder and leave out memory, knowledge and the code index

`DefaultRegistryConfig` gains `data_dir`, which overrides the Roder directory for memory, knowledge, the code index and default threads, and `memory`, `knowledge` and `code_index` switches (all on by default). A host that embeds Roder on someone's machine can keep its runs out of that person's own Roder data and recall context. The new fields break exhaustive struct literals, so the host's dependents are released against it.
