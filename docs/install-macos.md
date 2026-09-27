# Install on macOS

Install the native Intel or Apple Silicon build:

```sh
curl -fsSL https://raw.githubusercontent.com/wpfyorg/better-jiotv-go/rust/scripts/install.sh | sh
```

The default location is `$HOME/.local/bin`. Set `JIOTV_INSTALL_DIR` to change it, and add that directory to `PATH` if needed. Use `JIOTV_VARIANT=slim` for a headless install or `JIOTV_VERSION` to pin a release.

The release binaries are unsigned and macOS may show a Gatekeeper warning. Review the release checksum and use the system's normal approval process before launching. No built-in macOS service manager is provided; run `jiotv serve` in a terminal or configure a launchd job separately.

Data defaults to `$HOME/.jiotv_go`. Run `jiotv login otp` and `jiotv admin password` before starting the server. Update with `jiotv update`; uninstall by removing the executable and, if desired, the data directory.
