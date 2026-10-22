---
title: Update and Uninstall
description: Update JioTV safely and remove binaries or services while preserving data unless requested.
category: Maintenance
keywords: [update, upgrade, uninstall, remove, binary, version]
---

# Update and uninstall

Choose the update method that matches how JioTV was installed. Before updating, make a private backup of the data directory; see [Backup and restore](../maintenance-backup-restore/index.md).

## Update a standalone binary

Run:

```sh
jiotv update
```

To select a release, use `jiotv update --version v1.3.3` (replace with the desired version). The updater selects the same build variant and target as the running binary and checks the release checksum. Restart a systemd service after the update:

```sh
sudo systemctl restart jiotv
```

For a user service, use `systemctl --user restart jiotv`. The updater cannot replace a package-managed OpenWrt binary; update that installation through the package installer instead.

## Update Docker

Pull the image and recreate the container with the same options and persistent volume. See [Docker maintenance](../maintenance-docker/index.md) for commands.

## Uninstall

First stop and remove the service. For systemd, run `jiotv autostart remove` as the same user (or root) that installed it. For OpenWrt, remove the package with `apk del jiotv` or `opkg remove jiotv`; for Termux, remove the shell startup entry with `jiotv autostart remove`.

Then delete the `jiotv` executable from its installation directory. On Windows, remove `jiotv.exe` from the install directory and remove that directory from your user `PATH` if you no longer use it. For Docker, remove the container; remove its image if desired.

The data directory is separate from the executable and contains saved login state, password hash, access key and TLS files. Keep it if you may reinstall. Delete it only when you intend to permanently remove those settings and credentials. Docker data is in the named volume, and OpenWrt package removal preserves `/etc/jiotv` and `/etc/config/jiotv`.
