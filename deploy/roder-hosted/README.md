# Hosted Roder deployment examples

Secret-free deployment examples for the `roder-hosted` gateway
(`crates/roder-dist-hosted`). See `docs/roder-hosted-service.md` for the
configuration reference, security model, and operational guidance.

- `docker-compose.yml` — local/dev compose with an env-file secret and a
  private port (front with a TLS proxy).
- `systemd/roder-hosted.service` — hardened unit reading
  `/etc/roder/hosted.env` (chmod 600).
- `nomad/roder-hosted.nomad.hcl` — Docker driver job with Nomad-variable
  secrets.
- `kubernetes/roder-hosted.yaml` — Deployment/Service with a Secret-backed
  env and ConfigMap-mounted `hosted.toml`; add a TLS Ingress in front.

Common requirements, stated explicitly:

1. **TLS**: the gateway listens on plain WebSocket; always terminate TLS at
   a proxy/ingress and keep the listener private.
2. **Auth**: static keys are env references (`token_env`) resolved at
   startup; the service refuses to start when a referenced var is unset.
   Never write key material into these files.
3. **Data**: `data_root` holds per-tenant runtimes (thread stores,
   automation DBs) and the audit JSONL; mount it on a persistent volume and
   back it up per tenant directory.
