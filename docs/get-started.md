# Get started

For most people, installation is just one command. You do not need to know your CPU architecture or choose a release file yourself.

The normal `full` version includes the web UI and is the recommended choice.

## Linux, macOS, or Android / Termux

Open a terminal and run:

```sh
curl -fsSL https://raw.githubusercontent.com/wpfyorg/better-jiotv-go/rust/scripts/install.sh | sh
```

## Windows

Open PowerShell and run:

```powershell
irm https://raw.githubusercontent.com/wpfyorg/better-jiotv-go/rust/scripts/install.ps1 | iex
```

## OpenWrt

SSH into the router as `root` and run:

```sh
wget -qO- https://raw.githubusercontent.com/wpfyorg/better-jiotv-go/rust/scripts/install.sh | sh
```

The installer automatically detects `apk` or `opkg`, chooses the matching package for the router CPU, verifies its checksum, installs it, and enables the JioTV service.

## Docker

Run:

```sh
docker run -d --name jiotv --restart unless-stopped -p 5001:5001 -v jiotv-data:/app/.jiotv_go ghcr.io/wpfyorg/better-jiotv-go:latest
```

Docker chooses the correct CPU image automatically.

## Finish setup

For Linux, macOS, Windows, or Android / Termux:

```sh
jiotv login otp
jiotv admin password
jiotv serve --host 0.0.0.0 --port 5001
```

Enter the OTP yourself when prompted.

For OpenWrt, run the first two commands above and then start the service:

```sh
/etc/init.d/jiotv start
```

For Docker, run:

```sh
docker exec -it jiotv jiotv login otp
docker exec -it jiotv jiotv admin password
docker restart jiotv
```

Then open `http://<device-ip>:5001/` from another device on the same network.

## Optional: slim version

Most people should stay with the default `full` version. `slim` removes the web UI for headless IPTV setups.

On Linux, macOS, Android, or OpenWrt:

```sh
curl -fsSL https://raw.githubusercontent.com/wpfyorg/better-jiotv-go/rust/scripts/install.sh | JIOTV_VARIANT=slim sh
```

On Windows, see the [Windows guide](install-windows.md) only if you need advanced options such as `slim`, a pinned version, or a custom install folder.

For platform-specific updating, uninstalling, autostart, and troubleshooting, see [Installation](install.md).
