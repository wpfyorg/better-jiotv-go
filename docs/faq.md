# FAQ

## Which variant should I install?

Choose `full` for the built-in web UI. Choose `slim` for headless IPTV-only use.

## Which CPUs are supported?

See the platform table in [Get started](get-started.md). MIPS, MIPS64, ARMv6, and other unlisted targets are not currently release targets.

## Does the app manage Windows or macOS services?

No. Run it in a terminal or configure the platform's service manager separately. Linux systemd, Termux startup, and OpenWrt `procd` are documented where applicable.

## Where is application state stored?

By default it is under `~/.jiotv_go` (or the Windows profile directory if `HOME` is not set). Set `JIOTV_PATH_PREFIX` to choose a persistent path.

## Are the binaries signed?

The macOS and Windows binaries are currently unsigned. The installers verify release checksums.
