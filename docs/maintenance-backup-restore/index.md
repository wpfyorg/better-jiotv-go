---
title: Back Up and Restore JioTV
description: Protect JioTV credentials and settings with a stopped-service backup and restore.
category: Maintenance
keywords: [backup, restore, credentials, state, data directory, TLS key]
---

# Back up and restore JioTV

JioTV's data directory can contain provider sessions, the admin password hash, IPTV access key, device identity and TLS private key. Treat a backup as secret. Do not upload it to a public repository or share it in support logs.

## Find the data directory

The default is under the account's home directory. `JIOTV_PATH_PREFIX` or the `path_prefix` configuration setting can change it. OpenWrt uses `/etc/jiotv`; Docker stores it in `/app/.jiotv_go` inside the container, usually backed by a named volume. See [Data files, backups and logs](../configuration-storage-logs/index.md).

## Back up

Stop the server first so state files do not change during the copy. Use the relevant service or container controls, then copy the entire configured data directory to private storage while preserving file permissions. On OpenWrt, include both `/etc/jiotv` and `/etc/config/jiotv`. For Docker, back up the mounted volume while the container is stopped. Store backups encrypted or in a location accessible only to you.

## Restore

Stop JioTV, restore the complete directory and its permissions, and ensure the service uses the same `JIOTV_PATH_PREFIX`. On OpenWrt restore both the application data and UCI configuration. Start the service and confirm that admin login, provider account status, TLS access and playlist links work. Rotate the IPTV access key if its URL may have been exposed.
