---
title: Fix installation and startup problems
description: Diagnose unsupported downloads, missing files, permissions, dependencies and port conflicts.
category: Troubleshooting
keywords: [install, architecture, permissions, port, startup]
---

# Installation and startup problems

## Installer reports an unsupported platform or architecture

### Symptoms

The installer says the operating system or CPU is unsupported, or the downloaded program exits with an executable-format error.

### Possible causes

- The release does not include a build for this operating system/CPU combination.
- A 32-bit OS is being used on a 64-bit-capable device, or the wrong release asset was selected manually.
- An Android environment is being mistaken for ordinary Linux.

### Diagnosis

On Unix-like systems, check `uname -s` and `uname -m`, then compare them with the current platform table in [Getting started](../getting-started-requirements/index.md) or the platform installation guide. For Docker, inspect `docker info --format '{{.Architecture}}'` and confirm the image supports that architecture.

### Solution

Use the installer for your platform or select a release asset that matches both OS and architecture. On Android, follow the [Termux installation guide](../installations-termux/index.md). Do not force-install an incompatible binary.

### Verification

Run `jiotv --help`. It should print the command help and exit successfully. Then follow [Getting started](../getting-started-in-5-minutes/index.md) to start the server.

### Further help

Include OS, architecture, install method, exact error text and the release version. See [all installation guides](../installations/index.md).

## Download, checksum or missing-library errors

### Symptoms

The download fails, checksum verification rejects the file, startup reports a missing shared library, or the installer cannot write its destination.

### Possible causes

- Interrupted download or proxy/network filtering.
- A checksum mismatch or stale/manual download from a different release.
- A package or binary intended for another system.
- An unwritable install directory or missing runtime component for a manually built binary.

### Diagnosis

Retry the documented installer once and note the release and full error. If you downloaded an asset manually, compare it with the checksum published for that exact release. Check available space and the permissions of the documented install directory. Do not install arbitrary libraries suggested by an unrelated error page.

### Solution

Remove only the incomplete download or partial file you created, then use the official platform instructions to download again. If using a package manager, install the matching package and let the manager resolve its declared dependencies. For a source build, follow the repository build instructions for the required toolchain.

### Verification

Run `jiotv --help`, then consult the current [CLI options](../configuration-cli/index.md) for server flags. A successful help response confirms the executable starts; it does not confirm the server is reachable.

### Further help

Share OS version, architecture, install route and exact error. See [Updating](../maintenance-update/index.md) before replacing an existing install.

## Permission denied or state disappears after restart

### Symptoms

The program cannot create files, login state is lost, or the service works interactively but not as a service/container.

### Possible causes

- The service account cannot write to the configured data directory.
- The container data volume is missing or mounted at a different path.
- Interactive login and the running service use different `JIOTV_PATH_PREFIX` values or operating-system users.

### Diagnosis

Check the effective `JIOTV_PATH_PREFIX` and the service/container mount, then verify that the same runtime user can read and write there. The default data directory is under the user's home directory; Docker and OpenWrt may use a different location documented by their install guide.

### Solution

Use one persistent data directory and grant access to the service's own user. For Docker, keep the documented volume mounted when recreating the container. Do not recursively change ownership of broad system directories.

### Verification

Restart the service/container and confirm the web UI still recognizes its admin setup and JioTV login. Check **Settings** for account status.

### Further help

Provide install type, service user, configured data path and redacted startup error. Never send the data directory itself. See [Configuration](../configuration-reference/index.md) and the matching [platform guide](../installations/index.md).

## Address already in use or web UI unreachable

### Symptoms

Startup reports that binding failed, or another device cannot open the UI.

### Possible causes

- Another process already uses the configured port.
- The server is bound to loopback (`localhost`), which is reachable only on that machine.
- Host/container port mapping, LAN firewall, or address is wrong.
- The browser is using HTTP/HTTPS with the wrong listener.

### Diagnosis

Check startup output for the actual bind address and ports. The Rust CLI defaults to `localhost:5001`; TLS is opt-in and defaults to port `5443` when enabled. Check the host's process/socket tools for the selected port and inspect Docker port mappings or the platform service configuration.

### Solution

Stop only the duplicate JioTV instance you own, or choose an unused port using `--port`. To allow LAN clients, bind to the host's LAN interface or `0.0.0.0`, and allow only the needed LAN traffic in the firewall. For browser protected playback, enable TLS with `--tls`; see [protected playback](../troubleshooting-protected-playback/index.md). Keep IPTV clients on the documented HTTP playlist listener.

### Verification

Open the matching local URL on the server, then the LAN URL from another device. Confirm the startup address and port. Do not expose the service directly to the public internet; use a trusted private network or documented tunnel/access controls.

### Further help

Share bind error, OS, install type, configured host/ports and whether local access works. See [command-line options](../configuration-cli/index.md), [Docker](../installations-docker/index.md), and [network settings](../configuration-network-security/index.md).
