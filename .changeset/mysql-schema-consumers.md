---
roder-extension-host: major
roder-app-server: major
---

Use the MySQL session store with explicit release-time schema setup throughout
the extension host and app-server dependency graph. Run `roder-mysql-migrate`
before starting MySQL-backed workers; session startup only reads the schema
version. Embedders must use the matching MySQL store and extension-host versions
so configuration and runtime-owner types come from the same packages.
