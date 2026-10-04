# Updating

For raw binary installations, `jiotv update` downloads the matching platform and build variant, verifies its SHA-256 checksum, and replaces the executable. Restart the server or its service after updating. Use `jiotv update --version 1.3.1` to select a release.

Use the package manager to update OpenWrt installations (`apk` or `opkg`). Do not run the self-updater against a package-owned `/usr/bin/jiotv`. For Docker, pull and recreate the container while keeping its data volume.

The updater reads `JIOTV_UPDATE_REPO` and optional `JIOTV_UPDATE_TOKEN` for private release repositories. It preserves the `full` or `slim` variant.
