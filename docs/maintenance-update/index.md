---
title: How to Update JioTV
description: Safely update a standalone JioTV server, Docker container or OpenWrt package.
category: Maintenance
keywords: [update, upgrade, release, Docker, OpenWrt, backup, version]
---

# How to Update JioTV

Before updating, note how JioTV was installed and make a private [backup of its data](../maintenance-backup-restore/index.md). Your login sessions, admin password, access keys and TLS files live in the data directory, separately from the binary.

## Standalone Linux, macOS, Windows or Termux

1. Check the version shown in **Settings** or `jiotv --help` for available commands.
2. Stop the running foreground process or service. Record how it was started so you can use the same settings later.
3. Run `jiotv update` to fetch a matching release and verify its checksum. To choose a version, use `jiotv update --version v1.3.3` (replace with the release you intend to install).
4. Restart JioTV using your original command or service manager. On a systemd service, run `sudo systemctl restart jiotv` (or `systemctl --user restart jiotv` for a user service).
5. Open the app and check the version and whether Channels and Settings load.

**Expected result:** the new version runs with the existing configuration and account state. The updater needs access to its release source; private releases may require `JIOTV_UPDATE_TOKEN` configured privately.

## Docker

Pull the image used by your deployment and recreate the container using its existing environment and persistent volume. See [Docker maintenance](../maintenance-docker/index.md). Recreating a container without its prior volume can make data appear lost.

## OpenWrt

Use the OpenWrt package installer or package manager to update the package, then check the procd service. The in-app `jiotv update` command is not intended to replace a package-managed binary. See [OpenWrt maintenance](../maintenance-openwrt/index.md).

## If an update fails

Keep your original data backup. Collect the install method, exact version, error and relevant redacted logs. Avoid deleting data to fix an update. See [update troubleshooting](../troubleshooting-server-and-maintenance/index.md) and [uninstall safely](../maintenance-update-uninstall/index.md).
