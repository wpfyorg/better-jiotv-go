---
title: Fix Missing or Stale Programme Guide
description: Refresh the XMLTV guide when schedules are empty, old or mismatched.
category: Troubleshooting
keywords: [EPG, XMLTV, wrong programme, outdated, missing schedule, guide refresh]
---

# Fix Missing or Stale Programme Guide

## Symptoms
The IPTV guide is empty, shows yesterday's shows, has incorrect programmes, or the player reports **No guide for this channel**.

## Possible causes
EPG generation is disabled by default, provider guide data is unavailable, a source/account changed, or an IPTV player cached an old XMLTV copy.

## Diagnosis
1. Check **Settings** for the **EPG** status and copy the URL that your server displays.
2. Check whether `epg = true` or `JIOTV_EPG=true` is configured.
3. Compare the live player's programme guide to the external IPTV client's schedule.
4. Review the server output for guide-generation errors. New guide generation can take several minutes.

## Solution
1. Enable EPG if you want XMLTV and restart JioTV after the configuration change.
2. If generation needs a manual retry, stop a service using the same data directory before running `jiotv epg generate`.
3. Re-import or refresh the current playlist and XMLTV URLs in your IPTV application, especially after an account/Extras switch.
4. If only one channel lacks programme data, the provider may not supply a schedule for it.

## Verification
Confirm upcoming programme titles and times for more than one channel in the current account/source. If using an IPTV app, verify its cache refreshed.

## Further help
Provide EPG-enabled status, client name/version, affected channel names and redacted generation error. Do not share private playlist keys. See [EPG troubleshooting](../troubleshooting-epg-and-playlists/index.md).
