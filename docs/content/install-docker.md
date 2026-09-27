# Install with Docker

The GHCR image supports `linux/amd64`, `linux/arm64`, `linux/arm/v7`, and `linux/386`. Docker selects the matching image automatically.

```sh
docker run -d --name jiotv --restart unless-stopped \
  -p 5001:5001 \
  -v jiotv-data:/app/.jiotv_go \
  ghcr.io/wpfyorg/better-jiotv-go:latest
```

Use `ghcr.io/wpfyorg/better-jiotv-go:slim` for the headless build. Set `JIOTV_*` environment variables with Docker's `-e` options as needed. The persistent volume stores credentials and generated data.

Open `http://<docker-host-lan-ip>:5001/`. For first login, run `docker exec -it jiotv jiotv login otp` and `docker exec -it jiotv jiotv admin password`, then restart the container. Upgrade by pulling the new image and recreating the container with the same volume. Remove the container and image to uninstall; deleting the named volume also deletes saved credentials and settings.
