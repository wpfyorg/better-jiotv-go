---
title: Install with Docker
description: Run JioTV in Docker with persistent settings, credentials and TLS certificates.
category: Installation
keywords: [Docker, container, GHCR, linux, arm64, persistent volume]
---

# Install with Docker

The published image supports Linux `amd64`, `arm64`, ARMv7 and 386. Docker selects the matching architecture. The default image is `full`; the `slim` tag omits the browser interface.

Create and start a container with a persistent named volume:

```sh
docker run -d --name jiotv --restart unless-stopped \
  -p 5001:5001 -p 5443:5443 \
  -v jiotv-data:/app/.jiotv_go \
  ghcr.io/wpfyorg/better-jiotv-go:latest
```

The container starts the server on all interfaces with HTTPS enabled. On the Docker host, open `https://localhost:5443/`; from another trusted LAN device, use `https://<docker-host-ip>:5443/`. Accept the self-signed certificate warning. IPTV players use the HTTP playlist on port `5001`. Keep both the container and volume private; the volume contains credentials and the generated TLS key.

Set the admin password from the container's terminal, then sign in to the web interface and complete JioTV OTP login:

```sh
docker exec -it jiotv /usr/local/bin/jiotv admin password
```

View output with `docker logs -f jiotv`. For updates and backup instructions, see [Docker maintenance](../maintenance-docker/index.md).

To run headless, use `ghcr.io/wpfyorg/better-jiotv-go:slim`. To pass configuration, add supported options such as `-e JIOTV_TITLE="Living Room TV"` before the image name. Container arguments replace the image's default command, so preserve `--skip-update-check serve --host 0.0.0.0 --port 5001 --tls --tls-port 5443` when setting a custom command. See [CLI options](../configuration-cli/index.md).

## Verify and troubleshoot

Run `docker ps --filter name=jiotv` to confirm the container is running and `docker logs --tail 50 jiotv` to inspect startup messages. Open the HTTPS page at the host's configured port. If the port is occupied, choose a different host-side port mapping; if saved accounts disappear after recreation, confirm the `jiotv-data` volume was retained.

Use [Docker maintenance](../maintenance-docker/index.md) to pull updates, back up the volume and uninstall the container. For network errors see [Docker and web troubleshooting](../troubleshooting-server-and-maintenance/index.md).
