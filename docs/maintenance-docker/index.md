---
title: Docker Updates, Backups and Logs
description: Maintain a JioTV Docker container without losing its persistent data volume.
category: Maintenance
keywords: [Docker, update, backup, restore, logs, volume, container]
---

# Docker updates, backups and logs

The container writes persistent state to `/app/.jiotv_go`. Keep the named volume mounted whenever you recreate the container; it holds login state, settings and the self-signed TLS certificate.

## Logs and status

```sh
docker ps --filter name=jiotv
docker logs -f jiotv
```

Stop the container with `docker stop jiotv`. Start it again with `docker start jiotv`.

## Update

Pull the new image, remove the old container, and recreate it using the same run options and named volume:

```sh
docker pull ghcr.io/wpfyorg/better-jiotv-go:latest
docker stop jiotv
docker rm jiotv
```

Then rerun the `docker run` command from the [Docker installation guide](../installations-docker/index.md), retaining `-v jiotv-data:/app/.jiotv_go` and any ports or environment settings you had configured.

## Back up and restore

Stop the container before backing up. For a named volume called `jiotv-data`, an archive can be made with a temporary helper container:

```sh
docker run --rm -v jiotv-data:/data -v "$PWD:/backup" alpine \
  tar -czf /backup/jiotv-data.tar.gz -C /data .
```

Keep the archive private. To restore, stop JioTV, extract the archive into the same volume, then start or recreate the container. Confirm the volume name before running volume commands; removing a volume deletes its contents.

## Uninstall

Stop and remove the container with `docker stop jiotv && docker rm jiotv`. Remove the image with `docker image rm ghcr.io/wpfyorg/better-jiotv-go:latest` if wanted. Keep `jiotv-data` for a later reinstall. Remove the volume only when you intend to erase all saved settings and credentials.
