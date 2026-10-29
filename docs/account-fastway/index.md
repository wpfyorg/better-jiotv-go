---
title: Connect Fastway Live TV
description: Enable and connect the optional Fastway provider, and understand how its channels replace the JioTV catalogue.
category: Account Setup
keywords: [Fastway, provider, account, DASH, Widevine, PallyCon, channels]
---

# Connect Fastway Live TV

**Fastway** is an optional third-party live-TV source. It is separate from the JioTV account and from Extras, and it is disabled by default. A Fastway subscription is required; the server never grants or bypasses provider entitlements.

## Enable Fastway

Fastway is available in the standard build but stays off until the server configuration enables it:

```toml
# jiotv_go.toml
fastway = true
```

Or set `JIOTV_FASTWAY=true` for the process. Restart the server after changing the setting. When enabled, a **Fastway** card appears in **Settings**; see [Configuration Reference](../configuration-reference/index.md).

## Connect an account

Use the Fastway username and password for your subscription. This provider does not use the JioTV mobile-number OTP flow.

### Web interface

1. Open **Settings** and find the **Fastway** card.
2. Enter the Fastway username and password, then select **Connect**.
3. Check that the card says **Connected**. The header also shows a Fastway indicator while the provider is enabled and connected.

### Terminal

```bash
jiotv fastway login
```

The command prompts for the username and password and saves the login in the server's data directory. Restart a running server after a CLI login so it picks up the new credentials. To remove the saved login:

```bash
jiotv fastway logout
```

The password is kept locally because Fastway has no refresh-token endpoint; the server signs in again with the saved credentials when the provider rejects an expired access token. Do not share the data directory or its store file.

## What changes while Fastway is connected

- Fastway channels replace the JioTV catalogue in **Channels**, the M3U playlist and the XMLTV guide. Local custom channels remain available.
- If both Fastway and Extras are connected, Fastway takes priority. Disconnecting Fastway falls back to Extras, then to JioTV.
- Only channels the Fastway account is subscribed to are listed. A channel can still be unavailable if the provider rejects playback or the plan does not include it.
- Use **Fastway only** in **Channels** to narrow a mixed list, and look for the **Fastway** badge on channel tiles.
- Fastway playback is DASH. Protected channels use Widevine through the provider's PallyCon license service; the server proxies the license request but does not decrypt or bypass DRM. See [Protected playback troubleshooting](../troubleshooting-protected-playback/index.md).
- IPTV clients receive Fastway channels as `/live/mpd/...` entries with `inputstream.adaptive` and `manifest_type=mpd`.

Fastway does not use the Extras unlock-code gate. It is controlled by the `fastway` configuration flag and the saved login.

## Disconnect

Select **Disconnect** on the Fastway card, or run `jiotv fastway logout`. Disconnecting removes the saved Fastway login and returns the active catalogue to Extras or JioTV. If the server sets `disable_logout = true`, the web interface hides provider disconnect controls; the CLI command still works.

## Troubleshooting

- **The card is missing:** confirm `fastway = true` or `JIOTV_FASTWAY=true` is active for the running process, then restart.
- **Connect fails:** check the username and password with Fastway. The server reports the provider's message; it does not reveal whether the account has a subscription.
- **Channels are missing after connecting:** refresh the channel list. Only subscribed channels are shown, and the provider catalogue request may have failed temporarily.
- **Playback or license errors:** see [Fix DRM and Widevine playback errors](../fix-drm-playback-errors/index.md) and [Protected playback](../troubleshooting-protected-playback/index.md).
