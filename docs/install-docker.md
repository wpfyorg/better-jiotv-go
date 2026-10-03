# Install with Docker

The GHCR image supports `linux/amd64`, `linux/arm64`, `linux/arm/v7`, and `linux/386`. Docker selects the matching image automatically.

```sh
docker run -d --name jiotv --restart unless-stopped \
  -p 5001:5001 \
  -p 5443:5443 \
  -v jiotv-data:/app/.jiotv_go \
  ghcr.io/wpfyorg/better-jiotv-go:latest
```

Use `ghcr.io/wpfyorg/better-jiotv-go:slim` for the headless build. Set `JIOTV_*` environment variables with Docker's `-e` options as needed. The persistent volume stores credentials and generated data.

The image runs `serve --tls`: port 5443 is HTTPS with a self-signed certificate and port 5001 is plain HTTP. Browsers need HTTPS for protected playback, so open `https://<docker-host-lan-ip>:5443/` and accept the one-time certificate warning. IPTV apps should use the `http://<docker-host-lan-ip>:5001/` playlist. The certificate is stored in the data volume (`/app/.jiotv_go/tls/`), so keep the volume mounted or a new certificate (and a new warning) is created on each recreate. To use your own certificate, mount it and append `--tls-cert /certs/cert.pem --tls-key /certs/key.pem` to the command.

For first login, For first login, run `docker exec -it jiotv jiotv login otp` and `docker exec -it jiotv jiotv admin password`, then restart the container. Upgrade by pulling the new image and recreating the container with the same volume. Remove the container and image to uninstall; deleting the named volume also deletes saved credentials and settings.
