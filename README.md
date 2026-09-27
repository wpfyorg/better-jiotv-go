# JioTV in Rust

JioTV provides a local server for live TV, catch-up, EPG, IPTV playlists, and supported on-demand playback, with an optional web UI.

> **Acknowledgment**
>
> This project is a clean Rust rewrite inspired by the excellent [JioTV Go](https://github.com/jiotv-go/jiotv_go) project. JioTV Go established the original application, API behavior, playback flows, IPTV support, and much of the user experience that inspired this implementation.
>
> This rewrite started because we wanted to run the server directly on resource-constrained OpenWrt routers. The Go implementation was heavier than we wanted for that environment, which motivated a from-scratch Rust implementation focused on a smaller runtime footprint while preserving the useful behavior and ideas of the original project.
>
> Huge thanks to the JioTV Go maintainers and contributors for the work that made this project possible.

## Installation

Pick your platform and run one command. The installer detects your CPU automatically.

**Linux, macOS, Android/Termux**

```sh
curl -fsSL https://raw.githubusercontent.com/wpfyorg/better-jiotv-go/rust/scripts/install.sh | sh
```

**Windows PowerShell**

```powershell
irm https://raw.githubusercontent.com/wpfyorg/better-jiotv-go/rust/scripts/install.ps1 | iex
```

**OpenWrt**

```sh
wget -qO- https://raw.githubusercontent.com/wpfyorg/better-jiotv-go/rust/scripts/install.sh | sh
```

For Docker and step-by-step setup, see the [simple installation guide](https://wpfyorg.github.io/better-jiotv-go/).

See the [configuration reference](https://wpfyorg.github.io/better-jiotv-go/config.html) for `JIOTV_*` options.
