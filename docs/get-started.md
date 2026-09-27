# Get started

Choose `full` for the embedded web UI (recommended), or `slim` for a headless server. Both variants provide the IPTV server features.

## Linux, macOS, and Termux

```sh
curl -fsSL https://raw.githubusercontent.com/wpfyorg/better-jiotv-go/rust/scripts/install.sh | sh
```

Choose `slim`, pin a release, or select an install directory with environment variables:

```sh
curl -fsSL https://raw.githubusercontent.com/wpfyorg/better-jiotv-go/rust/scripts/install.sh -o install.sh
JIOTV_VARIANT=slim JIOTV_VERSION=1.1.0 JIOTV_INSTALL_DIR="$HOME/.local/bin" sh install.sh
```

The script checks the downloaded binary against the release `SHA256SUMS` file before installation.

## Windows PowerShell

```powershell
irm https://raw.githubusercontent.com/wpfyorg/better-jiotv-go/rust/scripts/install.ps1 | iex
```

For a headless install, use `-Variant slim`; the installer supports `-Version`, `-InstallDir`, and `-Repo`.

## OpenWrt

Install the matching package from GitHub Releases. Use `.apk` on apk-based OpenWrt and `.ipk` on opkg-based OpenWrt. See [OpenWrt installation](install-openwrt.md) for architecture detection and service setup.

## Docker

```sh
docker run -d --name jiotv --restart unless-stopped \
  -p 5001:5001 -v jiotv-data:/app/.jiotv_go \
  ghcr.io/wpfyorg/better-jiotv-go:latest
```

Use the `:slim` tag to omit the web UI. See [Docker installation](install-docker.md).

## Supported release targets

| Platform | Architecture | Release target |
|---|---|---|
| Linux | x86_64, aarch64, ARMv7, 386 | `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, `armv7-unknown-linux-musleabihf`, `i686-unknown-linux-musl` |
| macOS | Apple Silicon, Intel | `aarch64-apple-darwin`, `x86_64-apple-darwin` |
| Windows | x64, x86, ARM64 | `x86_64-pc-windows-msvc`, `i686-pc-windows-msvc`, `aarch64-pc-windows-msvc` |
| Android / Termux | ARM64, ARMv7, x86_64 | `aarch64-linux-android`, `armv7-linux-androideabi`, `x86_64-linux-android` |

The Unix installer detects the OS and architecture. For manual selection, `uname -m` reports the machine architecture; on Termux, use the Android target rather than a Linux target. UserLAnd distributions use their Linux userspace and Linux release target.

## First login

After installation, run these commands as the account that will own the server data:

```sh
jiotv login otp
jiotv admin password
jiotv serve --host 0.0.0.0 --port 5001
```

Enter the OTP yourself when prompted. Open `http://<server-lan-ip>:5001/` from a device on your network. Keep JioTV access-key and admin authentication enabled unless a trusted authentication layer protects the service.

See the guides for [Linux](install-linux.md), [macOS](install-macos.md), [Windows](install-windows.md), [Android](install-android.md), [Android TV](android-tv.md), [Docker](install-docker.md), [OpenWrt](install-openwrt.md), [SBCs](install-sbc.md), and [homelabs](install-homelab.md).
