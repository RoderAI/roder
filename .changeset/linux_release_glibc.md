---
roder: patch
---

# Run Linux release archives on Ubuntu 22.04 desktops

Build versioned and rolling Linux x86_64 and aarch64 archives in a pinned
Ubuntu 22.04 container on Ubuntu 24.04 hosts so they do
not require the newer GLIBC 2.38/2.39 symbols from an Ubuntu 24.04 build host.
Verify each Linux binary's GLIBC requirement, ACP startup and opt-in Cua tool
registration before packaging.
