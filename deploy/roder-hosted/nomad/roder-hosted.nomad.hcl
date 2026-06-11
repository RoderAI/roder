# Hosted Roder service — Nomad job example (roadmap phase 72).
# Secrets are injected from Vault/Nomad variables; never inline them here.
# Front the service with a TLS-terminating ingress (traefik/consul-ingress).

job "roder-hosted" {
  datacenters = ["dc1"]
  type        = "service"

  group "gateway" {
    count = 1

    network {
      port "ws" {
        to = 7900
      }
    }

    volume "data" {
      type   = "host"
      source = "roder-hosted-data"
    }

    task "roder-hosted" {
      driver = "docker"

      config {
        image   = "roder-hosted:latest"
        command = "roder-hosted"
        args    = ["--config", "/local/hosted.toml"]
        ports   = ["ws"]
      }

      volume_mount {
        volume      = "data"
        destination = "/var/lib/roder-hosted"
      }

      template {
        destination = "local/hosted.toml"
        data        = file("hosted.toml") # ship your validated config alongside the job
      }

      template {
        destination = "secrets/hosted.env"
        env         = true
        data        = <<EOT
RODER_HOSTED_KEY_ACME_ADMIN={{ with nomadVar "nomad/jobs/roder-hosted" }}{{ .acme_admin_key }}{{ end }}
EOT
      }

      resources {
        cpu    = 500
        memory = 1024
      }
    }
  }
}
