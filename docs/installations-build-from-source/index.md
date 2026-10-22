---
title: Build JioTV from Source
description: Compile the JioTV Rust server locally and optionally build OpenWrt packages.
category: Installation
keywords: [build from source, Rust, Cargo, OpenWrt SDK, compile]
---

# Build JioTV from source

Use this method when you need a local build or your system has no matching release binary. You need Git and a current stable Rust toolchain with Cargo. The full build is the default; the slim build excludes the embedded browser UI.

Clone the repository and compile:

```sh
git clone https://github.com/wpfyorg/better-jiotv-go.git
cd better-jiotv-go
cargo build --release
```

The binary is `target/release/jiotv`. To build slim:

```sh
cargo build --release --no-default-features --features slim
```

Run it in place to try it, or install it to a directory on `PATH`:

```sh
mkdir -p "$HOME/.local/bin"
install -m 0755 target/release/jiotv "$HOME/.local/bin/jiotv"
```

Cross-compiling requires a Rust target toolchain and linker suitable for the destination. The repository's release workflows define the supported target triples; a successful local build does not guarantee compatibility with a different libc or CPU. For OpenWrt, use the matching OpenWrt SDK and the supplied `scripts/build-openwrt-packages.sh`, which requires full and slim binaries plus the SDK path and version. Follow that script's usage line for its exact arguments.

See [first launch](../getting-started-first-launch/index.md), [CLI commands](../configuration-cli/index.md) and [update and uninstall](../maintenance-update-uninstall/index.md) for operating the compiled binary.

## Start, verify and maintain

Run `./target/release/jiotv --help` to verify the binary starts on the build machine. If the full build is installed on your `PATH`, run `jiotv serve` and browse to `http://localhost:5001/` on that machine. A slim build does not serve the browser UI.

To update a source installation, fetch the intended source revision and rebuild, then replace your installed executable while the server is stopped. To uninstall, remove the executable and any service registration you created. Keep the private data directory unless you mean to erase saved logins. See [startup troubleshooting](../troubleshooting-installation-startup/index.md).
