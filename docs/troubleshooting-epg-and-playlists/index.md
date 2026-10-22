---
title: Fix EPG, playlists and channel mode problems
description: Diagnose stale guide data, missing playlist channels, empty M3U output and source changes.
category: Troubleshooting
keywords: [EPG, XMLTV, M3U, playlist, missing channels, Extras, mode]
---

# EPG, playlists and source changes

## EPG is missing, empty or out of date

### Symptoms

The XMLTV URL fails, the guide is empty, or programme times do not match current listings.

### Possible causes

- EPG generation is disabled (`epg = false` by default).
- The guide has not finished generating or an upstream schedule request failed.
- The current account/source context changed, requiring a fresh guide.
- The IPTV application cached an older XMLTV file or maps channel IDs differently.

### Diagnosis

Check **Settings** for the EPG status and use the EPG URL shown there. Check the server output for generation errors. Confirm `JIOTV_EPG=true` or `epg = true` if using configuration. The server refreshes its EPG cache periodically; generation can take several minutes.

### Solution

Enable EPG and restart the server if needed. To explicitly regenerate from the CLI, stop the service first if it owns the same data directory, then run `jiotv epg generate` using the same config and data path. Re-import or refresh the XMLTV source in the IPTV app after generation. Do not delete the cache as a first step.

### Verification

Open the EPG URL shown in Settings or import it into the client again. Confirm programmes appear for a few channels and their times are plausible for the current schedule.

### Further help

Provide EPG enabled state, generation error and client name. Do not share a secret playlist URL. See [IPTV playlists](../using-iptv-playlists/index.md), [configuration](../configuration-reference/index.md), and [data and logs](../configuration-storage-logs/index.md).

## Playlist is empty or channels are missing

### Symptoms

An M3U import succeeds but has no entries, or the IPTV app shows fewer channels than the web UI.

### Possible causes

- The JioTV account is disconnected or the selected source has no channels.
- The IPTV client has an old playlist, a restrictive filter, or cached channel list.
- The wrong playlist URL or listener was copied.
- A channel is available only under a different account/source or entitlement.

### Diagnosis

Copy the playlist URL from **Settings → IPTV playlist**. Check account status and compare the web UI's Channels view with the IPTV app. Refresh the playlist and clear client-side group/search filters. Confirm you use the HTTP playlist URL and host/port displayed by the UI.

### Solution

Reconnect the relevant provider account, import the current playlist from Settings, and refresh the IPTV client's channel list. Select an appropriate channel source and filter. Do not hand-edit the access-key portion of the URL.

### Verification

Check that the new import includes channels visible in the web UI. Test one channel; a playlist import alone does not guarantee provider playback access.

### Further help

Share the player name, whether the web UI list is complete, and redacted server output. Never post the playlist URL because it contains an access key. See [IPTV playlists](../using-iptv-playlists/index.md) and [channel loading](../troubleshooting-channels-and-playback/index.md).

## EPG channel names do not match playlist channels

### Symptoms

The guide data exists but programmes do not appear beside channels in an IPTV app.

### Possible causes

- The client joins XMLTV guide entries and M3U channels using IDs/names that do not match.
- The playlist or guide came from different accounts/source modes or different refresh times.
- The client retained old channel mappings.

### Diagnosis

Refresh both playlist and guide from the URLs currently shown in Settings. Compare a channel's displayed name and guide mapping in the IPTV app. Check whether Extras is included in the playlist and whether the matching source/account is connected.

### Solution

Re-import both resources from the same server and refresh the client's guide mapping. If the client supports explicit XMLTV channel mapping, use its documented mapping controls. Do not alter server IDs or invent an unsupported mapping format.

### Verification

Confirm at least one known channel shows programmes after the client's next guide refresh.

### Further help

Include IPTV app/version, affected channel names (no secret links), and whether both URLs were refreshed. See [IPTV guide](../using-iptv-playlists/index.md).

## Channel list changes after switching account or Extras mode

### Symptoms

Channels or EPG entries disappear or change after JioTV login/logout, Extras login/logout, or locking Extras.

### Possible causes

- The active source catalogue changed with the account or Extras state.
- The IPTV playlist or guide has not been refreshed.
- The selected filter hides the newly active set.

### Diagnosis

Check **Settings** connection states and the Channels filters. The app combines available sources according to current login/configuration; switching an account invalidates account-specific cached data and triggers EPG regeneration when EPG is enabled.

### Solution

Confirm the intended account is connected and Extras is enabled/unlocked if required. Refresh Channels, then refresh the M3U and XMLTV data in each IPTV client. If the change is unexpected, restart only after checking logs and persistent state.

### Verification

Confirm source status and channel list in the web UI, then refresh the IPTV client and check a channel from the intended source.

### Further help

Report which source was changed, current Settings labels and which view is stale. See [account and Extras troubleshooting](../troubleshooting-accounts-and-extras/index.md), [Extras guide](../account-extras/index.md), and [IPTV playlists](../using-iptv-playlists/index.md).
