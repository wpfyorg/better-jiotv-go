# Install on Android / Termux

Install Termux from a trusted distribution source, open its shell, then run:

```sh
curl -fsSL https://raw.githubusercontent.com/wpfyorg/better-jiotv-go/main/scripts/install.sh | sh
```

The installer detects the Android ABI under Termux and selects ARM64, ARMv7, or x86_64. It installs under `$PREFIX/bin`. These native Android binaries target API 21. Set `JIOTV_VARIANT=slim` for headless use.

UserLAnd runs a Linux userspace rather than Termux: use its Linux package tools and Linux binary target. Do not use the Termux Android binary in a UserLAnd distribution.

Data defaults to `$HOME/.jiotv_go`. Run `jiotv login otp`, `jiotv admin password`, and `jiotv serve --tls`; then open `https://<device-ip>:5443/` and accept the one-time self-signed certificate warning (see [HTTPS in Usage](usage.md#https)). `jiotv autostart` adds startup to the Termux shell startup file. Android may stop background work under battery restrictions; exempt Termux from battery optimization for long-running use.

Update with `jiotv update`. Remove the binary from `$PREFIX/bin/jiotv`; remove the data directory only if saved credentials and settings should also be deleted.
