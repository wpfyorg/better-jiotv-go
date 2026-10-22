---
title: Fix web access, server stability, Docker and updates
description: Diagnose web UI access, settings, resource pressure, crashes, container state and update failures.
category: Troubleshooting
keywords: [web UI, settings, memory, crash, Docker, update]
---

# Web access, stability and maintenance

## Web UI does not open or Settings/API actions fail

### Symptoms

The address times out, the page is blank, login loops, or controls in Settings fail.

### Possible causes

- Server is stopped, listening on another address/port, or bound only to localhost.
- The full web UI is not included in a slim/headless build.
- Browser is using the wrong HTTP/HTTPS listener, or the connection is blocked by a firewall.
- A stale browser session or blocked cookie prevents admin access.

### Diagnosis

Check the server's startup output and use the exact address for its listener. Test from the server itself, then from a LAN device. Check **Settings** only after signing in with the web admin password. If the page shell loads but an action fails, note the action and visible message and inspect redacted server output. Confirm whether the installation is full or slim.

### Solution

Start the server with the intended LAN bind address and port, and permit that traffic on the trusted LAN. Use the full build for the web UI. Sign in again using the admin password and the same hostname/protocol. For account settings, use JioTV or Extras OTP as appropriate; these are separate from the admin login.

### Verification

Open the UI locally and from the intended LAN client, sign in, then check **Settings** account and EPG status. Verify an action by its resulting status rather than by the button response alone.

### Further help

Include install type, full/slim variant, URL scheme and port (omit any key path), bind address and redacted error. See [network access](../configuration-network-security/index.md), [first launch](../getting-started-first-launch/index.md), and [account troubleshooting](../troubleshooting-accounts-and-extras/index.md).

## Memory use is high or the server becomes slow

### Symptoms

The host reports high memory use, the UI becomes slow, or the process is terminated under load.

### Possible causes

- Several active viewers or large concurrent media requests increase workload.
- The host or container has a low memory limit or is under pressure from other services.
- Repeated provider/network errors cause retries or slow requests.
- The symptom is attributed to JioTV without checking total host/container use.

### Diagnosis

Measure the process and overall host/container resource use while the issue occurs. Compare idle and active playback, and note viewer count and whether one channel or all channels are involved. Check system/container logs for an out-of-memory kill. The current web Settings screen does not provide a memory-tuning control.

### Solution

Reduce simultaneous streams or competing workload, or allocate appropriate capacity to the service. Fix a confirmed network/provider failure separately. Avoid changing undocumented cache or memory settings; no generic cache-clear command is a supported memory fix.

### Verification

Observe resource use during the same workload after the adjustment. Confirm the process stays running and the UI responds while testing playback.

### Further help

Provide platform, container memory limit, observed process/host memory, active viewer count, and whether the process was killed. Never include private data files. See [Docker guide](../installations-docker/index.md) and [logs](../configuration-storage-logs/index.md).

## Server crashes, exits or restarts unexpectedly

### Symptoms

The process exits, a service repeatedly restarts, or the UI becomes unavailable after a playback or account action.

### Possible causes

- Invalid command/configuration, unavailable port or unwritable data path.
- Host/container memory pressure or service-manager restart policy.
- A runtime error or external process termination.

### Diagnosis

Capture the process exit status and a short redacted server/service log around the event. Check whether the server was started interactively, through systemd/procd, or in Docker. Check host resource and service-manager status. If it is reproducible, record the specific action and whether one channel triggers it.

### Solution

Correct a confirmed port, config or directory permission issue. For a service, use that platform's documented status and log commands; for Docker, inspect the container logs and exit state. Restart only after preserving the useful error output. Do not remove the data directory as a crash workaround.

### Verification

Start the same installation again and confirm it remains up. Repeat the reproducing action once while watching logs and resource use.

### Further help

Provide version, platform, run method, exit status, reproduction steps and redacted logs. See [storage and logs](../configuration-storage-logs/index.md), [background services](../maintenance-background-service/index.md), and [Docker maintenance](../maintenance-docker/index.md).

## Docker container starts but cannot be reached or loses state

### Symptoms

The container is running but the UI is unreachable, browser playback fails, or login/settings disappear after recreation.

### Possible causes

- Required host ports were not published or the process binds to the wrong interface.
- HTTPS port 5443 was not published for secure browser playback.
- The persistent data volume is absent, mounted at the wrong container path, or changed during recreation.
- The image architecture or full/slim variant does not match the intended use.

### Diagnosis

Check `docker ps` for status and published ports, then `docker logs jiotv` for startup output. Check the container's volume mapping and that it persists at `/app/.jiotv_go` for the documented image. Compare the image variant with the web UI requirement. Use the LAN address of the Docker host, not the container's loopback address.

### Solution

Recreate with the documented HTTP mapping (`5001:5001`), HTTPS mapping (`5443:5443`) when browser protected playback is needed, and the same persistent data volume. Do not remove the volume while recreating. For a custom command or certificate, follow the [Docker installation guide](../installations-docker/index.md).

### Verification

Check the published ports, open the host's HTTPS address from a LAN device, and confirm Settings still shows the saved account state after a controlled container restart.

### Further help

Share image tag/variant, redacted `docker ps` port mapping, volume destination, container exit state and short logs. Omit environment values and playlist URLs. See [Docker installation](../installations-docker/index.md), [network settings](../configuration-network-security/index.md), and [data files](../configuration-storage-logs/index.md).

## Update fails or the service still runs the old version

### Symptoms

`jiotv update` fails, reports an unavailable release, or the version does not change after an update.

### Possible causes

- Network/API access to the release service failed or the requested version is unavailable.
- The install directory is not writable by the user running the updater.
- A service manager or container is still running its existing process/image.
- The installation is an OS package and should be updated by its package manager.

### Diagnosis

Check the exact updater message, current `jiotv --help`/version output, install method and permissions on the executable directory. Check whether the process needs a service restart. For Docker, compare the running image ID/tag with the image you pulled. On OpenWrt, identify whether the app came from `apk` or `opkg`.

### Solution

For a manually installed binary, run `jiotv update` with a user who can update that binary, then restart the service as indicated. For Docker, pull and recreate the container with the same data volume. For OpenWrt, update through the package manager rather than the built-in binary updater. Do not run platform installers over an existing managed service unless its guide directs you to.

### Verification

Check the installed version after restart/recreation and verify Settings/account state and one channel. For Docker, confirm the new image is running and the persistent volume remains attached.

### Further help

Provide install type, version before/after, exact redacted error and whether the service restarted. See [Updating](../maintenance-update/index.md), [Docker](../installations-docker/index.md), and [platform installation guides](../installations/index.md).
