# Troubleshooting

## Installer reports an unsupported platform

Check `uname -s` and `uname -m`. The project publishes only the architectures listed in [Get started](get-started.md). Android Termux must use an Android target; UserLAnd uses a Linux target.

## Checksum verification fails

Delete the partial download and retry. The installer refuses to install an asset that is absent from `SHA256SUMS` or whose hash differs. Check that the requested release exists and that `JIOTV_REPO` points to the correct repository.

## The server starts but is unreachable on the LAN

Bind to `0.0.0.0`, permit the port through the host firewall, and use the host's LAN address. For OpenWrt, use a LAN-only firewall rule. Do not expose the service on WAN.

## Login state is missing after restart

Check that `JIOTV_PATH_PREFIX` points to the same persistent directory for interactive login and the service. Container and router deployments must persist their data location.

## Playback or client-specific problems

Confirm the account is logged in and the upstream content is available in the official client. Check the application logs and [Configuration](config.md) before changing authentication or proxy settings.
