# Introduction

> **Acknowledgment**
>
> This project is a clean Rust rewrite inspired by the excellent [JioTV Go](https://github.com/jiotv-go/jiotv_go) project. JioTV Go established the original application, API behavior, playback flows, IPTV support, and much of the user experience that inspired this implementation.
>
> This rewrite started because we wanted to run the server directly on resource-constrained OpenWrt routers. The Go implementation was heavier than we wanted for that environment, which motivated a from-scratch Rust implementation focused on a smaller runtime footprint while preserving the useful behavior and ideas of the original project.
>
> Huge thanks to the JioTV Go maintainers and contributors for the work that made this project possible.

JioTV runs a local HTTP server that makes JioTV live channels, catch-up, EPG, and supported on-demand content available to a web browser and compatible IPTV clients. The `full` build includes the web interface. The `slim` build is intended for headless IPTV deployments.

The server uses provider-issued playback URLs and license services. Protected streams remain protected; this project does not extract Widevine keys or decrypt media.

Choose a platform in [Get started](get-started.md), then use [Configuration](config.md) and [IPTV, M3U, and XMLTV](iptv.md) for setup details.
