# Android TV

JioTV can run on an Android TV device through Termux where available, or on a nearby Android/Linux host. The server exposes a local web interface and IPTV endpoints; use a TV browser or an IPTV player that can reach the host over the same network.

Install the [Termux Android binary](install-android.md) on the hosting device, or follow the [Linux guide](install-linux.md) for UserLAnd or a separate host. Complete `jiotv login otp` and `jiotv admin password`, then run `jiotv serve --host 0.0.0.0 --port 5001`.

On the TV, open `http://<host-lan-ip>:5001/` or add the server's M3U URL to a compatible IPTV client. Keep the host awake and permit LAN access to port 5001. Do not expose the service directly to the public internet.
