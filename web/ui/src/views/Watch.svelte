<script>
  import { onDestroy } from "svelte";
  import { api, loadChannels, formatTime } from "../lib/api.js";
  import { createShakaPlayer, loadLiveSource, sourceResolutionFailure, widevineCapability } from "../lib/shakaPlayer.js";

  let { id } = $props();

  let quality = $state(localStorage.getItem("quality") || "auto");
  let channel = $state(null);
  let guide = $state([]);
  let guideError = $state("");
  let playerContainer = $state();
  let video = $state();
  let playerError = $state("");
  let playerFailure = $state("generic");
  let cleanup = null;
  let playbackGeneration = 0;

  function isNow(program) {
    const now = Date.now();
    return program?.startEpoch <= now && program?.endEpoch > now;
  }

  $effect(() => {
    try {
      localStorage.setItem("quality", quality);
    } catch {}
  });

  $effect(() => {
    const current = id;
    channel = null;
    guide = [];
    guideError = "";
    loadChannels()
      .then((list) => (channel = list.find((c) => c.id === current) ?? null))
      .catch(() => {});
    api(`/epg/${encodeURIComponent(current)}/0`)
      .then((d) => {
        const now = Date.now();
        guide = (d?.epg ?? []).filter((p) => p.endEpoch > now).slice(0, 8);
      })
      .catch((err) => (guideError = err.message));
  });

  async function start(channelID, q) {
    const generation = ++playbackGeneration;
    const isCurrent = () => generation === playbackGeneration;
    cleanup?.();
    cleanup = null;
    playerError = "";
    playerFailure = "generic";

    try {
      const d = await api(`/api/live/play/${encodeURIComponent(channelID)}?q=${q}`);
      if (!isCurrent()) return;
      const session = await createShakaPlayer(playerContainer, video);
      if (!isCurrent()) {
        await session.destroy().catch(() => {});
        return;
      }
      const player = session.player;
      cleanup = () => session.destroy().catch(() => {});
      const drmCapability = d.dash && d.license ? await widevineCapability() : null;
      if (!isCurrent()) return;

      if (d.license) {
        player.configure({
          drm: {
            servers: { "com.widevine.alpha": d.license },
            advanced: { "com.widevine.alpha": { videoRobustness: "SW_SECURE_CRYPTO", audioRobustness: "SW_SECURE_CRYPTO" } },
          },
          streaming: { bufferBehind: 2, bufferingGoal: 6, rebufferingGoal: 2 },
        });
      }

      await loadLiveSource({
        player,
        video,
        source: d,
        drmCapability,
        isCurrent,
        onTerminalError: (message, info) => {
          playerError = message;
          playerFailure = info?.kind ?? "generic";
        },
      });
    } catch (err) {
      if (isCurrent()) {
        playerError = err.message || String(err);
        playerFailure = sourceResolutionFailure(err);
      }
    }
  }

  $effect(() => {
    if (playerContainer && video) start(id, quality);
  });

  onDestroy(() => {
    playbackGeneration++;
    cleanup?.();
  });
</script>

<div class="layout">
  <div class="stage" bind:this={playerContainer}>
    <!-- svelte-ignore a11y_media_has_caption -->
    <video bind:this={video} autoplay playsinline></video>
    <div class="player-meta" aria-hidden="true">
      <span class="live-pill"><span></span>LIVE</span>
      <div class="player-copy">
        <strong>{channel?.name ?? id}</strong>
        {#if guide[0] && isNow(guide[0])}<small>{guide[0].showname}</small>{/if}
      </div>
    </div>
    {#if playerError && playerFailure === "browser_unsupported"}
      <div class="player-error player-overlay" role="alert" data-playback-state="browser_unsupported">
        <h2>Not playable in this browser</h2>
        <p>
          {#if playerError.startsWith("DRM_ENVIRONMENT_BLOCKED")}
            This browser has no working Widevine DRM module, which this channel's protected stream needs.
          {:else}
            This channel's Widevine-protected stream uses a video format (typically HEVC) that this browser cannot decrypt and play.
          {/if}
          It can still play in IPTV apps such as TiviMate on an Android TV device with hardware DRM, using the M3U playlist.
        </p>
        <a class="overlay-action" href="#/settings">Get the playlist URL</a>
        <small>{playerError}</small>
      </div>
    {:else if playerError && playerFailure === "provider_unavailable"}
      <div class="player-error player-overlay" role="alert" data-playback-state="provider_unavailable">
        <h2>Stream unavailable from provider</h2>
        <p>The provider is not serving this channel right now. Try again later or pick another channel.</p>
        <a class="overlay-action" href="#/">All channels</a>
        <small>{playerError}</small>
      </div>
    {:else if playerError}
      <p class="player-error" role="alert" data-playback-state="generic">{playerError}</p>
    {/if}
  </div>

  <aside>
    <div class="channel-card">
      {#if channel}<img src={channel.logo} alt="" />{/if}
      <div class="channel-copy">
        <h1>{channel?.name ?? id}</h1>
        <p class="muted">
          {[channel?.category, channel?.language].filter(Boolean).join(" · ")}
          {#if channel?.extras}<span class="badge extras">Extra</span>{/if}
          {#if channel?.requiresSubscription}<span class="badge premium">Premium</span>{/if}
        </p>
        {#if channel?.requiresSubscription}<p class="subscription-notice" role="note">A subscription may be required to play this channel.</p>{/if}
      </div>
    </div>

    <section class="quality-block" aria-label="Playback quality">
      <div class="section-title"><span>Quality</span><small class="muted">{quality === "auto" ? "Adaptive" : quality}</small></div>
      <div class="quality-options">
        {#each ["auto", "high", "medium", "low"] as option}
          <button class:active={quality === option} aria-pressed={quality === option} onclick={() => (quality = option)}>
            {option === "auto" ? "Auto" : option[0].toUpperCase() + option.slice(1)}
          </button>
        {/each}
      </div>
    </section>

    <div class="section-title guide-title"><span>Program guide</span><small class="muted">Live schedule</small></div>
    {#if guideError}
      <p class="empty muted">No guide for this channel.</p>
    {:else if guide.length === 0}
      <p class="empty muted">Loading schedule…</p>
    {:else}
      <ol class="guide">
        {#each guide as p, i}
          <li class:now={isNow(p)}>
            <span class="time">
              {#if isNow(p)}<span class="status-dot"></span>{/if}
              {formatTime(p.startEpoch)}
            </span>
            <span class="program-copy">
              <span class="program-line">
                <strong>{p.showname}</strong>
                {#if isNow(p)}<em>NOW</em>{:else if i === 1}<em class="next">NEXT</em>{/if}
              </span>
              {#if isNow(p) && p.description}<span class="desc muted">{p.description}</span>{/if}
            </span>
          </li>
        {/each}
      </ol>
    {/if}
    <a class="back-link" href="#/"><span>←</span> All channels</a>
  </aside>
</div>

<style>
  .layout { display: grid; gap: 24px; grid-template-columns: minmax(0, 1fr) minmax(310px, 360px); align-items: start; }
  .stage {
    position: relative;
    aspect-ratio: 16 / 9;
    background: #000;
    border: 1px solid color-mix(in srgb, var(--border) 82%, transparent);
    border-radius: 16px;
    overflow: hidden;
    box-shadow: 0 22px 60px rgba(0, 0, 0, .28);
  }
  video { width: 100%; height: 100%; display: block; background: #000; }
  .player-meta {
    position: absolute;
    z-index: 2;
    top: 16px;
    left: 16px;
    display: flex;
    align-items: center;
    gap: 10px;
    max-width: min(70%, 520px);
    padding: 8px 11px;
    border: 1px solid rgba(255, 255, 255, .11);
    border-radius: 12px;
    color: #fff;
    background: rgba(8, 10, 14, .62);
    backdrop-filter: blur(12px);
    pointer-events: none;
  }
  .live-pill {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    flex: 0 0 auto;
    padding: 3px 7px;
    border-radius: 999px;
    font-size: 10px;
    font-weight: 800;
    letter-spacing: .08em;
    background: rgba(220, 38, 38, .92);
  }
  .live-pill span { width: 5px; height: 5px; border-radius: 50%; background: #fff; box-shadow: 0 0 0 3px rgba(255, 255, 255, .15); }
  .player-copy { min-width: 0; display: flex; flex-direction: column; line-height: 1.2; }
  .player-copy strong, .player-copy small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .player-copy strong { font-size: 13px; }
  .player-copy small { margin-top: 2px; color: rgba(255, 255, 255, .72); font-size: 11px; }
  .player-error {
    position: absolute;
    z-index: 4;
    right: 16px;
    bottom: 68px;
    max-width: min(80%, 560px);
    margin: 0;
    padding: 9px 12px;
    border: 1px solid color-mix(in srgb, var(--danger) 45%, transparent);
    border-radius: 10px;
    color: #fff;
    background: color-mix(in srgb, #1a0d10 92%, transparent);
    box-shadow: 0 10px 30px rgba(0, 0, 0, .32);
    font-size: 13px;
  }
  .player-error.player-overlay {
    inset: 0;
    z-index: 5;
    max-width: none;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 24px;
    border: 0;
    border-radius: 0;
    box-shadow: none;
    text-align: center;
    /* Always dark like the video stage, so the white text works in both themes. */
    background: rgba(8, 10, 16, .94);
    overflow-y: auto;
  }
  .player-overlay h2 { margin: 0; font-size: 20px; letter-spacing: -.01em; }
  .player-overlay p { margin: 0; max-width: 460px; color: rgba(255, 255, 255, .8); font-size: 13.5px; line-height: 1.5; }
  .player-overlay small { color: rgba(255, 255, 255, .62); font-size: 11px; }
  .overlay-action {
    padding: 9px 16px;
    border-radius: var(--radius);
    color: var(--accent-text);
    background: var(--accent);
    text-decoration: none;
    font-size: 13px;
    font-weight: 650;
  }
  .overlay-action:hover { filter: brightness(1.08); }
  .overlay-action:focus-visible { outline: 0; box-shadow: var(--focus); }

  /* Shaka UI theme: builds on the shared overrides in app.css using app tokens. */
  .stage { font-family: inherit; }
  .stage :global(.shaka-controls-container) {
    background: linear-gradient(to top, rgba(0, 0, 0, .86) 0%, rgba(0, 0, 0, .4) 22%, transparent 55%);
  }
  .stage :global(.shaka-controls-button-panel > button) {
    min-width: 40px;
    height: 40px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: calc(var(--radius) - 2px);
    transition: background .15s ease, color .15s ease;
  }
  .stage :global(.shaka-controls-button-panel > button:hover) { background: rgba(255, 255, 255, .16); }
  .stage :global(.shaka-controls-button-panel > button:active) { background: rgba(255, 255, 255, .24); }
  .stage :global(.shaka-controls-container button:focus-visible),
  .stage :global(.shaka-controls-container input:focus-visible) {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .stage :global(.shaka-current-time) { color: rgba(255, 255, 255, .88); font-size: 13px; }
  .stage :global(.shaka-range-container) { border-radius: 999px; background: rgba(255, 255, 255, .22); }
  .stage :global(.shaka-range-element::-webkit-slider-thumb) {
    width: 13px;
    height: 13px;
    background: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 30%, transparent);
  }
  .stage :global(.shaka-range-element::-moz-range-thumb) {
    width: 13px;
    height: 13px;
    background: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 30%, transparent);
  }
  .stage :global(.shaka-volume-bar-container) { width: 84px; }
  .stage :global(.shaka-overflow-menu),
  .stage :global(.shaka-settings-menu) {
    background: color-mix(in srgb, var(--surface) 96%, transparent);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 4px;
    backdrop-filter: blur(14px);
  }
  .stage :global(.shaka-overflow-menu button),
  .stage :global(.shaka-settings-menu button) {
    min-height: 36px;
    color: var(--text);
    border-radius: calc(var(--radius) - 4px);
    font-family: inherit;
  }
  .stage :global(.shaka-overflow-menu button:hover),
  .stage :global(.shaka-settings-menu button:hover),
  .stage :global(.shaka-overflow-menu button:focus-visible),
  .stage :global(.shaka-settings-menu button:focus-visible) { background: var(--surface-2); }
  .stage :global(.shaka-overflow-menu .material-icons-round),
  .stage :global(.shaka-settings-menu .material-icons-round) { color: var(--muted); }
  .stage :global(.shaka-settings-menu span[aria-selected="true"]),
  .stage :global(.shaka-settings-menu button[aria-selected="true"] span) { color: var(--accent); font-weight: 650; }
  .stage :global(.shaka-spinner-path) { stroke: var(--accent); }
  .stage :global([class*="shaka-tooltip"]:hover::after),
  .stage :global([class*="shaka-tooltip"]:focus-visible::after) {
    background: color-mix(in srgb, var(--surface) 94%, transparent);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  aside { display: flex; flex-direction: column; gap: 20px; min-width: 0; padding-top: 2px; }
  .channel-card {
    display: flex;
    gap: 12px;
    align-items: center;
    padding: 10px;
    border: 1px solid var(--border);
    border-radius: 14px;
    background: color-mix(in srgb, var(--surface) 88%, transparent);
  }
  .channel-card img { width: 58px; height: 42px; flex: 0 0 auto; object-fit: contain; background: var(--surface-2); border-radius: 10px; padding: 6px; }
  .channel-copy { min-width: 0; }
  h1 { margin: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 20px; line-height: 1.2; letter-spacing: -.02em; }
  .badge.premium { color: var(--danger); border-color: color-mix(in srgb, var(--danger) 45%, transparent); }
  .subscription-notice { margin: 8px 0 0; padding: 8px 10px; border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent); border-radius: 8px; color: var(--danger); font-size: 13px; }
  h1 + p { display: flex; align-items: center; gap: 6px; margin: 4px 0 0; font-size: 12px; }
  .section-title { display: flex; align-items: center; justify-content: space-between; gap: 10px; font-size: 12px; font-weight: 750; letter-spacing: .035em; }
  .section-title > span { color: var(--text); }
  .section-title small { font-size: 11px; font-weight: 500; text-transform: capitalize; letter-spacing: 0; }
  .quality-block { display: flex; flex-direction: column; gap: 9px; }
  .quality-options {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    padding: 3px;
    border: 1px solid var(--border);
    border-radius: 11px;
    background: var(--surface);
  }
  .quality-options button {
    min-width: 0;
    padding: 7px 5px;
    border: 0;
    border-radius: 8px;
    color: var(--muted);
    background: transparent;
    cursor: pointer;
    font-size: 11px;
    font-weight: 650;
  }
  .quality-options button:hover { color: var(--text); }
  .quality-options button.active { color: var(--text); background: var(--surface-2); box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--border) 75%, transparent); }
  .guide-title { margin-top: 2px; }
  .empty { margin: -6px 0 0; padding: 12px; border: 1px dashed var(--border); border-radius: 11px; font-size: 12px; text-align: center; }
  .guide { list-style: none; margin: -8px 0 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
  .guide li { display: grid; grid-template-columns: 62px minmax(0, 1fr); gap: 9px; padding: 9px 10px; border: 1px solid transparent; border-radius: 11px; font-size: 13px; }
  .guide li.now { border-color: color-mix(in srgb, var(--accent) 24%, var(--border)); background: color-mix(in srgb, var(--surface-2) 86%, var(--accent) 14%); }
  .time { display: flex; align-items: flex-start; gap: 6px; padding-top: 1px; color: var(--muted); font-variant-numeric: tabular-nums; font-size: 12px; }
  .status-dot { width: 6px; height: 6px; margin-top: 5px; border-radius: 50%; background: #ef4444; box-shadow: 0 0 0 3px rgba(239, 68, 68, .13); }
  .program-copy { min-width: 0; }
  .program-line { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .program-line strong { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 13px; line-height: 1.35; }
  .program-line em { flex: 0 0 auto; padding: 1px 5px; border-radius: 999px; color: #fff; background: #dc2626; font-size: 8px; font-style: normal; font-weight: 800; letter-spacing: .06em; }
  .program-line em.next { color: var(--muted); background: var(--surface); }
  .desc { display: -webkit-box; overflow: hidden; margin-top: 4px; font-size: 11.5px; line-height: 1.45; -webkit-box-orient: vertical; -webkit-line-clamp: 2; }
  .back-link {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 7px;
    margin-top: 1px;
    padding: 8px 10px;
    border: 1px solid transparent;
    border-radius: 10px;
    color: var(--muted);
    text-decoration: none;
    font-size: 12px;
    font-weight: 600;
  }
  .back-link:hover { color: var(--text); border-color: var(--border); background: var(--surface); }

  @media (max-width: 1050px) {
    .layout { grid-template-columns: 1fr; gap: 20px; }
    aside { width: min(100%, 760px); }
  }

  @media (orientation: landscape) and (max-height: 500px) {
    /* Leave room for the sticky header so the whole player and its controls stay on screen. */
    .stage { width: min(100%, calc((100dvh - 100px) * 16 / 9)); margin-inline: auto; }
  }
  @media (max-width: 420px) {
    .stage .player-error.player-overlay { padding: 12px; gap: 6px; }
    .stage .player-overlay small { display: none; }
  }
  @media (max-width: 640px) {
    .layout { gap: 14px; }
    .stage { border-radius: 12px; box-shadow: 0 14px 36px rgba(0, 0, 0, .22); }
    .player-meta { top: 10px; left: 10px; max-width: calc(100% - 20px); padding: 6px 8px; border-radius: 9px; }
    .live-pill { padding: 2px 6px; font-size: 9px; }
    .player-copy strong { font-size: 12px; }
    .player-copy small { display: none; }
    .player-error { right: 10px; bottom: 54px; max-width: calc(100% - 20px); font-size: 11px; }
    .player-error.player-overlay { inset: 0; max-width: none; padding: 16px; gap: 8px; font-size: 11px; }
    .player-overlay h2 { font-size: 16px; }
    .player-overlay p { font-size: 12px; line-height: 1.4; }
    .stage :global(.shaka-controls-button-panel > button) { min-width: 44px; height: 44px; }
    .stage :global(.shaka-overflow-menu button),
    .stage :global(.shaka-settings-menu button) { min-height: 44px; }
    .stage :global(.shaka-volume-bar-container) { display: none; }
    aside { gap: 16px; padding-top: 0; }
    .channel-card { padding: 8px; }
    .channel-card img { width: 52px; height: 38px; }
    h1 { font-size: 18px; }
    .quality-options button { padding: 7px 3px; font-size: 10.5px; }
    .guide li { grid-template-columns: 56px minmax(0, 1fr); padding: 8px; }
    .desc { -webkit-line-clamp: 3; }
  }
</style>
