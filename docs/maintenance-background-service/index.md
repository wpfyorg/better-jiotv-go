---
title: Run JioTV as a Background Service
description: Configure supported systemd or Termux startup and inspect service status and logs.
category: Maintenance
keywords: [service, systemd, autostart, background, logs, Termux]
---

# Run JioTV as a background service

`jiotv autostart` supports Linux systems running systemd and Termux. OpenWrt uses its package-managed procd service. Windows and macOS do not have a service installer in this project; run JioTV in a terminal there.

## Linux with systemd

Run as your normal user to create a user service, or with `sudo` to create a system-wide service. Include the server arguments you need:

```sh
sudo env JIOTV_PATH_PREFIX=/var/lib/jiotv jiotv autostart --args "--host 0.0.0.0 --port 5001 --tls"
```

For a user service, use `jiotv autostart --args "--host 0.0.0.0 --tls"`. The command enables and starts the service. A user service may need lingering enabled to run at boot without an active login; the command prints the appropriate hint.

Check status and logs:

```sh
sudo systemctl status jiotv
sudo journalctl -u jiotv -f
```

For a user service, add `--user` to `systemctl` and run `journalctl --user -u jiotv -f`. Remove the service with `jiotv autostart remove` under the same account that installed it. The generated environment file may contain secrets; keep its permissions private.

## Termux

`jiotv autostart` adds a marked command to Termux's `bash.bashrc` to start JioTV in the background when a shell opens. Remove it with `jiotv autostart remove`. Android power management can still stop background work.

## OpenWrt

Use `/etc/init.d/jiotv start|stop|restart|enable|disable` to control the package service. Read its output with `logread -e jiotv`. Configuration is in `/etc/config/jiotv`; commit UCI changes before restarting.
