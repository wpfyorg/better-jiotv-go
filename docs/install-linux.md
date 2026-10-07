# Install on Linux

The one-line installer detects x86_64, aarch64, ARMv7, and 32-bit x86 Linux and verifies the downloaded release checksum:

```sh
curl -fsSL https://raw.githubusercontent.com/wpfyorg/better-jiotv-go/main/scripts/install.sh | sh
```

The default is `full`. Set `JIOTV_VARIANT=slim` for a headless installation. Set `JIOTV_INSTALL_TLS=0` to omit the HTTPS instructions from the installer output. Set `JIOTV_VERSION`, `JIOTV_REPO`, or `JIOTV_INSTALL_DIR` to select a release or install location. Without an override, the installer uses `/usr/local/bin` when run as root and writable, otherwise `$HOME/.local/bin`.

Data defaults to `$HOME/.jiotv_go`; set `JIOTV_PATH_PREFIX` for a service-managed location. Run `jiotv login otp`, then `jiotv admin password`, and start with `jiotv serve --host 0.0.0.0 --tls`. For systemd use `jiotv autostart --args "--host 0.0.0.0 --port 5001 --tls"`; see [Homelab / VM / LXC](install-homelab.md) for a dedicated service user example.

On the same machine, open the UI at `https://localhost:5443/` (self-signed certificate; accept the one-time browser warning) because browsers need HTTPS or `localhost` for protected playback. From other devices use `https://<host>:5443/`, and give IPTV apps the `http://<host>:5001/` playlist, where `<host>` is the machine's address on your network (for example from `hostname -I`). See [HTTPS in Usage](usage.md#https).

Update a manually installed binary with `jiotv update`. Remove by stopping/removing its service, deleting the `jiotv` executable, and removing the data directory only if you intend to delete saved credentials and settings.

Linux uses systemd autostart where available. OpenWrt uses its package-managed `procd` service instead. ARMv6 and MIPS are not release targets.
