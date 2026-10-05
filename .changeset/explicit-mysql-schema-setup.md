---
roder-ext-mysql-session: major
---

Move MySQL schema setup out of runtime connections. Run `roder-mysql-migrate`
with `RODER_MYSQL_SESSION_URL` as a release step before starting workers.
Session-store connections now check the schema version with a primary-key read
and never acquire migration locks or issue DDL.
