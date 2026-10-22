---
title: Install JioTV with Docker Compose
description: Use a Compose file to run and update JioTV with persistent account data.
category: Installation
keywords: [Docker Compose, compose.yaml, Docker, volume, container, update, logs]
---

# Install JioTV with Docker Compose

Docker Compose keeps the container ports and data volume in one reusable file. Install Docker Engine or Docker Desktop with the **Docker Compose** plugin first. Check that `docker compose version` works. The released image supports Linux `amd64`, `arm64`, ARMv7 and 386; Docker selects the correct supported image architecture.

## 1. Create the Compose file

Make a new directory such as `jiotv-compose` and save this content as **compose.yaml**:

```yaml
services:
  jiotv:
    image: ghcr.io/wpfyorg/better-jiotv-go:latest
    restart: unless-stopped
    ports:
      - "5001:5001"
      - "5443:5443"
    volumes:
      - jiotv-data:/app/.jiotv_go

volumes:
  jiotv-data:
```

The `latest` image includes the web UI. Use the `slim` tag only when you want IPTV access without the browser interface. The named volume stores credentials, configuration and local TLS files. Keep it private.

## 2. Start and check the service

From the directory containing **compose.yaml**, run:

```sh
docker compose config
docker compose up -d
docker compose ps
docker compose logs --tail 50 jiotv
```

**Expected result:** `docker compose ps` shows a running `jiotv` service. On the host, open **https://localhost:5443/**; from a trusted network device, use `https://<docker-host-ip>:5443/`. A browser warning about the generated self-signed certificate is expected. Port 5001 serves plain HTTP for compatible IPTV clients.

## 3. Set the admin password and connect JioTV

The first-run server can provide a private web setup URL in its logs. Do not share that link: it contains an access key. You can instead set the password interactively from your Compose directory:

```sh
docker compose exec jiotv /usr/local/bin/jiotv admin password
docker compose restart jiotv
```

Then open the web page, sign in with the admin password and connect your provider account in **Settings** using its OTP form. See [first launch](../getting-started-first-launch/index.md) and [account connection](../account-connect-jiotv/index.md).

## 4. Update and inspect the container

Stop playback before updating, then run:

```sh
docker compose pull
docker compose up -d
docker compose ps
docker compose logs --tail 50 jiotv
```

**Expected result:** Compose recreates the service when the image has changed and keeps the same `jiotv-data` volume. Reopen Settings to confirm the account is still connected. See [Docker maintenance](../maintenance-docker/index.md) to back up the volume before major updates.

## 5. Stop or uninstall

Run `docker compose stop` for a temporary stop, `docker compose start` to resume, or `docker compose down` to remove the container and its network while keeping the saved volume. **Do not use `docker compose down -v`** unless you deliberately want to delete the saved JioTV accounts, password and settings. See [Docker troubleshooting](../troubleshooting-server-and-maintenance/index.md) for port conflicts and missing data.
