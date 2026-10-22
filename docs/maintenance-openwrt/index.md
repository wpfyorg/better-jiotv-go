---
title: Maintain JioTV on OpenWrt
description: Update, inspect, back up and remove the JioTV OpenWrt package and service.
category: Maintenance
keywords: [OpenWrt, update, apk, opkg, backup, logread, uninstall]
---

# Maintain JioTV on OpenWrt

## Update

Run the [OpenWrt installer](../installations-openwrt/index.md) again. It downloads the package for the detected package manager and architecture, verifies its checksum, then updates through `apk` or `opkg`. Do not run `jiotv update` on a package installation.

## Service and logs

```sh
/etc/init.d/jiotv status
/etc/init.d/jiotv restart
logread -e jiotv
```

Boot startup is controlled by `/etc/init.d/jiotv enable` and `disable`. The service configuration is `/etc/config/jiotv`; after UCI edits, use `uci commit jiotv` and restart the service.

## Back up and uninstall

Stop the service and privately copy `/etc/jiotv` and `/etc/config/jiotv`. These contain credentials, TLS key material and settings. Package removal with `apk del jiotv` or `opkg remove jiotv` preserves these paths. Use `jiotv-slim` in the removal command if that is the installed package.

To erase JioTV data after removing the package, separately delete `/etc/jiotv` and `/etc/config/jiotv` only after confirming that you no longer need their contents.
