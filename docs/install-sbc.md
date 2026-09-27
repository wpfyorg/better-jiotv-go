# Install on Raspberry Pi and other SBCs

This guide is for Raspberry Pi OS, Debian, Ubuntu, Armbian, and similar systemd-based Linux distributions running on an SBC.

The `full` build is recommended unless the machine is intentionally headless/IPTV-only.

## Pick the right binary

```sh
uname -m
```

| SBC OS/CPU | Target |
|---|---|
| 64-bit Raspberry Pi OS, arm64 Debian/Ubuntu/Armbian | `aarch64-unknown-linux-musl` |
| 32-bit ARMv7 OS | `armv7-unknown-linux-musleabihf` |
| x86_64 SBC | `x86_64-unknown-linux-musl` |

ARMv6 boards such as the original Raspberry Pi and Pi Zero are not in the current release matrix.

## Download and verify

The built-in updater defaults to `wpfyorg/better-jiotv-go`; change `REPO` if your release is published elsewhere.

```sh
REPO=wpfyorg/better-jiotv-go
VARIANT=full

case "$(uname -m)" in
  x86_64) TARGET=x86_64-unknown-linux-musl ;;
  aarch64|arm64) TARGET=aarch64-unknown-linux-musl ;;
  armv7l|armv7*) TARGET=armv7-unknown-linux-musleabihf ;;
  *) echo "unsupported release architecture: $(uname -m)"; exit 1 ;;
esac

ASSET="jiotv-${VARIANT}-${TARGET}"
curl -fL -o "${ASSET}" "https://github.com/${REPO}/releases/latest/download/${ASSET}"
curl -fL -o SHA256SUMS "https://github.com/${REPO}/releases/latest/download/SHA256SUMS"
grep "  ${ASSET}$" SHA256SUMS | sha256sum -c -
sudo install -m 0755 "${ASSET}" /usr/local/bin/jiotv
jiotv --help
```

If the release repository is private, use authenticated GitHub download or build the binary locally instead of the unauthenticated `curl` commands above.

## Persistent state and first login

For a system-wide service, keep state under `/var/lib/jiotv`:

```sh
sudo install -d -m 0700 /var/lib/jiotv
sudo env JIOTV_PATH_PREFIX=/var/lib/jiotv /usr/local/bin/jiotv login otp
sudo env JIOTV_PATH_PREFIX=/var/lib/jiotv /usr/local/bin/jiotv admin password
```

Enter the OTP yourself when the login command prompts for it.

## Install the systemd service

The binary can create the service for you. Environment variables beginning with `JIOTV_` are persisted into the service environment file.

```sh
sudo env \
  JIOTV_PATH_PREFIX=/var/lib/jiotv \
  JIOTV_EPG=true \
  /usr/local/bin/jiotv autostart --args "--host 0.0.0.0 --port 5001"
```

Then check:

```sh
systemctl status jiotv
journalctl -u jiotv -f
```

Open `http://SBC_LAN_IP:5001/` from another device.

## User service instead of root

If you install the binary and run `jiotv autostart` as your normal login user, it creates a systemd user unit instead. Follow the command's printed `loginctl enable-linger` hint if you want that service to survive logout and start at boot.

## Raspberry Pi notes

- Raspberry Pi 4/5 running a 64-bit OS should use the `aarch64` release even though the CPU may also support 32-bit software.
- Raspberry Pi 2/3 on a 32-bit ARMv7 userspace should use the ARMv7 release.
- Pi Zero 2 W can use the aarch64 release only when the installed OS/userspace is 64-bit.
- Original Pi/Pi Zero ARMv6 systems need a separate build; no release binary is produced for them today.

## Updating

```sh
sudo /usr/local/bin/jiotv update
sudo systemctl restart jiotv
```

For a private release repository, provide `JIOTV_UPDATE_TOKEN` to the update command.
