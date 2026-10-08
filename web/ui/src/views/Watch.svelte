<script>
  import { onDestroy, onMount } from "svelte";
  import { api, loadChannels, formatTime } from "../lib/api.js";
  import {
    createShakaPlayer,
    loadLiveSource,
    playbackHttpStatus,
    sourceDenialStatus,
    sourceResolutionFailure,
    widevineCapability,
  } from "../lib/shakaPlayer.js";

  let { id } = $props();

  let quality = $state(localStorage.getItem("quality") || "auto");
  let channel = $state(null);
  let guide = $state([]);
  let guideError = $state("");
  let playerContainer = $state();
  let video = $state();
  let playerState = $state({ kind: "loading", eyebrow: "Live TV", title: "Starting live TV…", detail: "" });
  let showMeta = $state(true);
  let clock = $state(Date.now());
  let programToast = $state("");
  let paused = $state(false);
  let muted = $state(false);
  let showQualityMenu = $state(false);
  let showMoreMenu = $state(false);
  let cleanup = null;
  let cleanupChain = Promise.resolve();
  let runID = 0;
  let activityTimer = null;
  let toastTimer = null;

  const fullStageKinds = new Set(["offline", "playback", "secure", "protected", "subscription", "restricted", "extras-signin", "extras-status", "extras-unavailable", "service"]);

  let currentProgramIndex = $derived(guide.findIndex((program) => isNow(program, clock)));
  let currentProgram = $derived(currentProgramIndex >= 0 ? guide[currentProgramIndex] : null);
  let programProgress = $derived.by(() => {
    if (!currentProgram) return 0;
    const duration = currentProgram.endEpoch - currentProgram.startEpoch;
    if (duration <= 0) return 0;
    return Math.max(0, Math.min(100, ((clock - currentProgram.startEpoch) / duration) * 100));
  });


  function isNow(program, now = Date.now()) {
    return program?.startEpoch <= now && program?.endEpoch > now;
  }

  function setPlayerState(kind, eyebrow, title, detail = "") {
    playerState = { kind, eyebrow, title, detail };
  }

  function stopActiveSession() {
    const stop = cleanup;
    cleanup = null;
    const previous = cleanupChain;
    cleanupChain = (async () => {
      await previous.catch(() => {});
      await stop?.();
    })();
    return cleanupChain;
  }

  function showPlaybackFailure(kind, message = "", status = null) {
    if (!navigator.onLine) {
      setPlayerState("offline", "Connection issue", "You’re offline", "Reconnect to the internet, then try the stream again.");
      return;
    }

    if (kind === "insecure_context") {
      setPlayerState("secure", "Secure connection", "Needs a secure connection", "Protected and encrypted streams require HTTPS or localhost in a browser. Open the app over HTTPS, then try again.");
      return;
    }
    if (kind === "browser_unsupported") {
      const detail = message.startsWith("DRM_ENVIRONMENT_BLOCKED")
        ? "This browser does not have a working Widevine module for this protected stream. Try a supported browser or device."
        : "This browser cannot decode this protected stream. Try a supported browser or device, or use the playlist in an IPTV app.";
      setPlayerState("protected", "Protected playback", "Protected playback unavailable", detail);
      return;
    }
    if (kind === "provider_unavailable") {
      setPlayerState("service", "Live TV", "Stream unavailable from provider", "The provider is not serving this channel right now. Try again later or choose another channel.");
      return;
    }
    if (kind === "provider_denied") {
      const providerStatus = status ? ` Provider response: HTTP ${status}.` : "";
      if (channel?.requiresSubscription) {
        setPlayerState("subscription", "Account access", "Subscription required", `The provider refused this premium channel. The current account may not include it.${providerStatus}`);
      } else {
        setPlayerState("restricted", "Account access", "The provider refused playback", `The current account may not have access, or its session may need refreshing.${providerStatus}`);
      }
      return;
    }
    setPlayerState("playback", "Playback issue", "Playback unavailable", "The live stream could not be started. Check the connection or try again in a moment.");
  }

  function markActivity() {
    showMeta = true;
    clearTimeout(activityTimer);
    activityTimer = setTimeout(function hideMeta() {
      if (playerContainer?.contains(document.activeElement)) {
        activityTimer = setTimeout(hideMeta, 3000);
        return;
      }
      showMeta = false;
    }, 3000);
  }

  function handlePlayerKeydown(event) {
    markActivity();
    if (event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey) return;
    const target = event.target;
    if (
      target instanceof HTMLButtonElement ||
      target instanceof HTMLAnchorElement ||
      target instanceof HTMLInputElement ||
      target instanceof HTMLSelectElement ||
      target instanceof HTMLTextAreaElement ||
      target?.isContentEditable
    ) return;

    switch (event.key.toLowerCase()) {
      case " ":
      case "k":
        event.preventDefault();
        togglePlayback();
        break;
      case "m":
        event.preventDefault();
        toggleMute();
        break;
      case "f":
        event.preventDefault();
        toggleFullscreen();
        break;
    }
  }

  function formatElapsed(ms) {
    const seconds = Math.max(0, Math.floor(ms / 1000));
    const minutes = Math.floor(seconds / 60);
    return `${String(minutes).padStart(2, "0")}:${String(seconds % 60).padStart(2, "0")}`;
  }

  async function togglePlayback() {
    if (!video) return;
    markActivity();
    if (video.paused) {
      try {
        await video.play();
      } catch {
        showPlaybackFailure("generic");
      }
    } else {
      video.pause();
    }
  }

  function toggleMute() {
    if (!video) return;
    video.muted = !video.muted;
    muted = video.muted;
    markActivity();
  }

  async function toggleFullscreen() {
    markActivity();
    try {
      if (document.fullscreenElement) {
        await document.exitFullscreen?.();
      } else if (video?.webkitDisplayingFullscreen) {
        video.webkitExitFullscreen?.();
      } else if (playerContainer?.requestFullscreen) {
        await playerContainer.requestFullscreen();
      } else {
        video?.webkitEnterFullscreen?.();
      }
    } catch {}
  }

  async function togglePictureInPicture() {
    showMoreMenu = false;
    markActivity();
    try {
      if (document.pictureInPictureElement) {
        await document.exitPictureInPicture?.();
      } else {
        await video?.requestPictureInPicture?.();
      }
    } catch {}
  }

  function chooseQuality(option) {
    quality = option;
    showQualityMenu = false;
    markActivity();
  }

  function updateClock() {
    const previousKey = currentProgram ? `${currentProgram.startEpoch}:${currentProgram.endEpoch}` : "";
    const nextClock = Date.now();
    const nextProgram = guide.find((program) => isNow(program, nextClock)) ?? null;
    const nextKey = nextProgram ? `${nextProgram.startEpoch}:${nextProgram.endEpoch}` : "";
    clock = nextClock;
    if (previousKey && nextKey && previousKey !== nextKey) {
      programToast = nextProgram?.showname || "";
      clearTimeout(toastTimer);
      toastTimer = setTimeout(() => (programToast = ""), 4200);
    }
  }

  async function tapToPlay() {
    if (!video) return;
    video.muted = false;
    try {
      await video.play();
      setPlayerState("playing", "", "", "");
      markActivity();
    } catch {
      showPlaybackFailure("generic");
    }
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
    programToast = "";
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
    const thisRun = ++runID;
    const isCurrent = () => thisRun === runID;
    await stopActiveSession();
    if (!isCurrent()) return;
    setPlayerState("loading", "Live TV", "Starting live TV…", "");
    paused = false;
    muted = Boolean(video?.muted);
    showQualityMenu = false;
    showMoreMenu = false;
    markActivity();

    try {
      if (channelID.startsWith("ex_")) {
        const status = await api("/api/extras/status").catch(() => null);
        if (!isCurrent()) return;
        if (!status) {
          if (!navigator.onLine) {
            setPlayerState("offline", "Connection issue", "You’re offline", "Reconnect to the internet, then try the stream again.");
          } else {
            setPlayerState("extras-unavailable", "Extras", "Extras status unavailable", "We could not check the extras account right now. Wait a moment and try again.");
          }
          return;
        }
        if (!status?.extras?.enabled) {
          setPlayerState("extras-status", "Extras", "Extras is not enabled", "Enable extras in account settings, then return here to start playback.");
          return;
        }
        if (!status?.extras?.connected) {
          setPlayerState("extras-signin", "Extras", "Connect extras to play", "This channel comes from extras. Connect the extras account, then return here to start playback.");
          return;
        }
      }

      const source = await api(`/api/live/play/${encodeURIComponent(channelID)}?q=${q}`);
      if (!isCurrent()) return;
      const session = await createShakaPlayer(playerContainer, video, { controls: false });
      if (!isCurrent()) {
        await session.destroy().catch(() => {});
        return;
      }
      const player = session.player;
      const onPlaying = () => {
        if (isCurrent()) {
          paused = false;
          setPlayerState("playing", "", "", "");
        }
      };
      const onPause = () => {
        if (isCurrent()) paused = true;
      };
      const onVolumeChange = () => {
        if (isCurrent()) muted = Boolean(video?.muted);
      };
      const onWaiting = () => {
        if (isCurrent() && !fullStageKinds.has(playerState.kind)) setPlayerState("buffering", "Live TV", "Buffering…", "");
      };
      video.addEventListener("playing", onPlaying);
      video.addEventListener("pause", onPause);
      video.addEventListener("volumechange", onVolumeChange);
      video.addEventListener("waiting", onWaiting);
      cleanup = async () => {
        video?.removeEventListener("playing", onPlaying);
        video?.removeEventListener("pause", onPause);
        video?.removeEventListener("volumechange", onVolumeChange);
        video?.removeEventListener("waiting", onWaiting);
        await session.destroy().catch(() => {});
      };

      const drmCapability = source.dash && source.license ? await widevineCapability() : null;
      if (!isCurrent()) return;
      if (source.license) {
        player.configure({
          drm: {
            servers: { "com.widevine.alpha": source.license },
            advanced: { "com.widevine.alpha": { videoRobustness: "SW_SECURE_CRYPTO", audioRobustness: "SW_SECURE_CRYPTO" } },
          },
          streaming: { bufferBehind: 2, bufferingGoal: 6, rebufferingGoal: 2 },
        });
      }

      player.addEventListener("buffering", (event) => {
        if (!isCurrent() || fullStageKinds.has(playerState.kind)) return;
        const buffering = event?.buffering ?? event?.detail?.buffering;
        setPlayerState(buffering ? "buffering" : "playing", "Live TV", buffering ? "Buffering…" : "", "");
      });

      await loadLiveSource({
        player,
        video,
        source,
        drmCapability,
        isCurrent,
        allowMutedFallback: false,
        onAutoplayResult: (played) => {
          if (isCurrent()) setPlayerState(played ? "playing" : "tap", "", played ? "" : "Tap to play", "");
        },
        onTerminalError: (message, info) => {
          if (!isCurrent()) return;
          const status = playbackHttpStatus(info?.hlsError ?? info?.dashError);
          showPlaybackFailure(info?.kind ?? "generic", message, status);
        },
      });
    } catch (err) {
      if (isCurrent()) {
        showPlaybackFailure(sourceResolutionFailure(err), err?.message || String(err), sourceDenialStatus(err));
      }
    }
  }

  $effect(() => {
    if (playerContainer && video) start(id, quality);
  });

  onMount(() => {
    const clockTimer = setInterval(updateClock, 1000);
    const onOffline = () => {
      runID += 1;
      video?.pause();
      setPlayerState("offline", "Connection issue", "You’re offline", "Reconnect to the internet, then try the stream again.");
      void stopActiveSession();
    };
    const onOnline = () => {
      setPlayerState("reconnecting", "Live TV", "Reconnecting live stream…", "");
      start(id, quality);
    };
    window.addEventListener("offline", onOffline);
    window.addEventListener("online", onOnline);
    window.addEventListener("keydown", markActivity);
    markActivity();
    return () => {
      clearInterval(clockTimer);
      window.removeEventListener("offline", onOffline);
      window.removeEventListener("online", onOnline);
      window.removeEventListener("keydown", markActivity);
    };
  });

  onDestroy(() => {
    runID += 1;
    clearTimeout(activityTimer);
    clearTimeout(toastTimer);
    video?.pause();
    void stopActiveSession();
  });
</script>

<div class="layout">
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div
    class:has-overlay={fullStageKinds.has(playerState.kind)}
    class="stage"
    role="region"
    aria-label="Live player"
    tabindex="0"
    bind:this={playerContainer}
    onpointermove={markActivity}
    onpointerdown={markActivity}
    ontouchstart={markActivity}
    onkeydown={handlePlayerKeydown}
    onfocusin={markActivity}
  >
    <!-- svelte-ignore a11y_media_has_caption -->
    <video bind:this={video} autoplay playsinline></video>
    {#if showMeta && playerState.kind === "playing"}
      <div class="player-controls">
        <div class="controls-scrim" aria-hidden="true"></div>
        <div class="player-meta" aria-hidden="true">
          <div class="player-meta-top">
            <span class="live-pill"><span></span>LIVE</span>
            <strong>{channel?.name ?? id}</strong>
          </div>
          {#if currentProgram}<strong class="player-program">{currentProgram.showname}</strong>{/if}
          {#if currentProgram?.description}<span class="player-program-description">{currentProgram.description}</span>{/if}
        </div>
        <div class="player-seek" aria-hidden="true"><span style:width={`${programProgress}%`}></span></div>
        <div class="player-control-row">
          <div class="player-control-left">
            <button class="icon-control" onclick={togglePlayback} aria-label={paused ? "Resume live playback" : "Pause live playback"}>
              {#if paused}
                <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m8 5 11 7-11 7V5Z" /></svg>
              {:else}
                <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 5h4v14H7zM13 5h4v14h-4z" /></svg>
              {/if}
            </button>
            <button class="icon-control" onclick={toggleMute} aria-label={muted ? "Unmute" : "Mute"}>
              {#if muted}
                <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 9v6h4l5 4V5L8 9H4Z" /><path fill="none" d="m17 9 4 6M21 9l-4 6" /></svg>
              {:else}
                <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 9v6h4l5 4V5L8 9H4Z" /><path fill="none" d="M16 9.5a4 4 0 0 1 0 5M18.5 7a7.5 7.5 0 0 1 0 10" /></svg>
              {/if}
            </button>
            <span class="player-live-clock">LIVE <span aria-hidden="true">•</span> {formatElapsed(currentProgram ? clock - currentProgram.startEpoch : 0)}</span>
          </div>
          <div class="player-control-right">
            <div class="control-menu-wrap">
              <button class="quality-control" onclick={() => { showQualityMenu = !showQualityMenu; showMoreMenu = false; markActivity(); }} aria-haspopup="menu" aria-expanded={showQualityMenu}>
                {quality === "auto" ? (channel?.hd ? "HD" : "Auto") : quality[0].toUpperCase() + quality.slice(1)}
              </button>
              {#if showQualityMenu}
                <div class="control-menu quality-menu" role="menu" aria-label="Playback quality">
                  {#each ["auto", "high", "medium", "low"] as option}
                    <button class:active={quality === option} role="menuitemradio" aria-checked={quality === option} onclick={() => chooseQuality(option)}>{option === "auto" ? "Auto" : option[0].toUpperCase() + option.slice(1)}</button>
                  {/each}
                </div>
              {/if}
            </div>
            <div class="control-menu-wrap">
              <button class="icon-control" onclick={() => { showMoreMenu = !showMoreMenu; showQualityMenu = false; markActivity(); }} aria-label="More player options" aria-haspopup="menu" aria-expanded={showMoreMenu}>
                <svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="5" r="1.5" /><circle cx="12" cy="12" r="1.5" /><circle cx="12" cy="19" r="1.5" /></svg>
              </button>
              {#if showMoreMenu}
                <div class="control-menu more-menu" role="menu" aria-label="More player options">
                  {#if video?.requestPictureInPicture || document.pictureInPictureElement}
                    <button role="menuitem" onclick={togglePictureInPicture}>Picture in picture</button>
                  {/if}
                  <button role="menuitem" onclick={() => { showMoreMenu = false; start(id, quality); }}>Restart stream</button>
                </div>
              {/if}
            </div>
            <button class="icon-control" onclick={toggleFullscreen} aria-label="Toggle fullscreen">
              <svg viewBox="0 0 24 24" aria-hidden="true"><path fill="none" d="M8 3H3v5M16 3h5v5M8 21H3v-5M16 21h5v-5" /></svg>
            </button>
          </div>
        </div>
      </div>
    {/if}
    {#if ["loading", "buffering", "reconnecting"].includes(playerState.kind)}
      <div class="player-progress" role="status" aria-live="polite">
        <span class="spinner" aria-hidden="true"></span>
        <span>{playerState.title}</span>
      </div>
    {:else if playerState.kind === "tap"}
      <button class="tap-to-play" onclick={tapToPlay} aria-label="Start live playback">
        <span aria-hidden="true">▶</span>
        Tap to play
      </button>
    {/if}
    {#if programToast}
      <div class="program-toast" role="status" aria-live="polite">
        <span class="live-pill"><span></span>LIVE</span>
        <span class="toast-copy"><small>Now playing</small><strong>{programToast}</strong></span>
      </div>
    {/if}
    {#if fullStageKinds.has(playerState.kind)}
      <section class="player-error" role="alert" aria-labelledby="player-error-title">
        <span class="error-mark" aria-hidden="true">!</span>
        <p class="error-eyebrow">{playerState.eyebrow}</p>
        <h2 id="player-error-title">{playerState.title}</h2>
        <p class="error-detail">{playerState.detail}</p>
        <div class="error-actions">
          {#if ["extras-signin", "extras-status", "secure", "protected"].includes(playerState.kind)}
            <a class="retry-button" href="#/settings">Open account settings</a>
          {:else}
            <button class="retry-button" onclick={() => start(id, quality)}>Try again</button>
          {/if}
          <a class="channels-button" href="#/">All channels</a>
        </div>
      </section>
    {/if}
  </div>

  <aside>
    <div class="channel-card">
      {#if channel}<img src={channel.logo} alt="" />{/if}
      <div class="channel-copy">
        <h1>{channel?.name ?? id}</h1>
        <p class="muted">
          {[channel?.category, channel?.language].filter(Boolean).join(" · ")}
          {#if channel?.requiresSubscription}<span class="badge premium">Premium</span>{/if}
          {#if channel?.extras}<span class="badge extras">Extra</span>{/if}
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
          <li class:now={isNow(p, clock)}>
            <span class="time">
              {#if isNow(p, clock)}<span class="status-dot"></span>{/if}
              {formatTime(p.startEpoch)}
            </span>
            <span class="program-copy">
              <span class="program-line">
                <strong>{p.showname}</strong>
                {#if isNow(p, clock)}<em>NOW</em>{:else if i === currentProgramIndex + 1}<em class="next">NEXT</em>{/if}
              </span>
              {#if isNow(p, clock) && p.description}<span class="desc muted">{p.description}</span>{/if}
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
    isolation: isolate;
    aspect-ratio: 16 / 9;
    background: #000;
    border: 1px solid color-mix(in srgb, var(--border) 82%, transparent);
    border-radius: 16px;
    overflow: hidden;
    box-shadow: 0 22px 60px rgba(0, 0, 0, .28);
  }
  video { width: 100%; height: 100%; display: block; background: #000; }
  .player-controls {
    position: absolute;
    z-index: 12;
    inset: 0;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    padding: 0 24px 23px;
    pointer-events: none;
    color: #fff;
  }
  .controls-scrim {
    position: absolute;
    z-index: -1;
    inset: 0;
    background: linear-gradient(180deg, transparent 58%, rgba(0, 0, 0, .16) 68%, rgba(0, 0, 0, .72) 100%);
  }
  .player-meta {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    width: min(76%, 700px);
    margin-bottom: 16px;
    text-shadow: 0 1px 4px rgba(0, 0, 0, .7);
  }
  .player-meta-top {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .player-meta-top > strong {
    overflow: hidden;
    color: #d4dae4;
    font-size: 11px;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .player-program {
    max-width: 100%;
    overflow: hidden;
    font-size: 20px;
    line-height: 1.15;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .player-program-description {
    width: min(100%, 481px);
    overflow: hidden;
    color: #8c96a8;
    font-family: Inter, system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
    font-size: 11px;
    font-weight: 400;
    line-height: 1.35;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .live-pill {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    flex: 0 0 auto;
    padding: 3px 7px;
    border-radius: 999px;
    font-size: 9px;
    font-weight: 800;
    letter-spacing: .04em;
    background: rgba(220, 38, 38, .92);
  }
  .live-pill span { width: 5px; height: 5px; border-radius: 50%; background: #fff; }
  .player-seek {
    width: 100%;
    height: 3px;
    overflow: hidden;
    margin-bottom: 14px;
    border-radius: 999px;
    background: rgba(255, 255, 255, .22);
  }
  .player-seek span { display: block; height: 100%; border-radius: inherit; background: #6c8fff; }
  .player-control-row { display: flex; align-items: center; justify-content: space-between; min-height: 52px; gap: 16px; }
  .player-control-left, .player-control-right { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .player-control-row button { pointer-events: auto; }
  .icon-control, .quality-control {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    height: 38px;
    min-width: 38px;
    padding: 0;
    border: 1px solid transparent;
    border-radius: 10px;
    color: #fff;
    background: transparent;
    cursor: pointer;
    font: inherit;
  }
  .icon-control:hover, .quality-control:hover, .icon-control:focus-visible, .quality-control:focus-visible { border-color: rgba(255, 255, 255, .14); background: rgba(255, 255, 255, .08); }
  .icon-control:focus-visible, .quality-control:focus-visible { outline: 2px solid #fff; outline-offset: 2px; }
  .icon-control svg { width: 22px; height: 22px; fill: currentColor; stroke: currentColor; stroke-width: 1.8; stroke-linecap: round; stroke-linejoin: round; }
  .icon-control svg path[fill="none"] { fill: none; }
  .player-live-clock { margin-left: 2px; color: #d4dae4; font-size: 11px; font-weight: 600; letter-spacing: .01em; white-space: nowrap; }
  .quality-control { height: 30px; min-width: 38px; padding: 0 9px; border-color: rgba(255, 255, 255, .13); background: rgba(255, 255, 255, .09); font-size: 10px; font-weight: 800; }
  .control-menu-wrap { position: relative; pointer-events: auto; }
  .control-menu {
    position: absolute;
    right: 0;
    bottom: calc(100% + 8px);
    display: flex;
    min-width: 150px;
    flex-direction: column;
    gap: 2px;
    padding: 6px;
    border: 1px solid rgba(255, 255, 255, .12);
    border-radius: 10px;
    background: rgba(8, 10, 14, .96);
    box-shadow: 0 14px 36px rgba(0, 0, 0, .34);
    backdrop-filter: blur(14px);
  }
  .control-menu button { min-height: 34px; padding: 0 10px; border: 0; border-radius: 7px; color: #d4dae4; background: transparent; text-align: left; font: inherit; font-size: 11px; cursor: pointer; }
  .control-menu button:hover, .control-menu button.active { color: #fff; background: rgba(255, 255, 255, .08); }
  .player-progress {
    position: absolute;
    z-index: 8;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    color: rgba(255, 255, 255, .82);
    background: linear-gradient(180deg, rgba(0, 0, 0, .12), rgba(0, 0, 0, .24));
    pointer-events: none;
    font-size: 12px;
    font-weight: 650;
  }
  .spinner { width: 15px; height: 15px; border: 2px solid rgba(255, 255, 255, .24); border-top-color: #fff; border-radius: 50%; animation: spin .7s linear infinite; }
  .tap-to-play {
    position: absolute;
    z-index: 9;
    left: 50%;
    top: 50%;
    transform: translate(-50%, -50%);
    display: inline-flex;
    align-items: center;
    gap: 9px;
    min-height: 44px;
    padding: 0 18px;
    border: 1px solid rgba(255, 255, 255, .18);
    border-radius: 12px;
    color: #fff;
    background: rgba(8, 10, 14, .74);
    backdrop-filter: blur(12px);
    cursor: pointer;
    font: inherit;
    font-size: 13px;
    font-weight: 750;
  }
  .tap-to-play:hover { background: rgba(18, 21, 28, .9); }
  .program-toast {
    position: absolute;
    z-index: 10;
    left: 16px;
    bottom: 70px;
    display: flex;
    align-items: center;
    gap: 10px;
    max-width: min(72%, 520px);
    padding: 9px 11px;
    border: 1px solid rgba(255, 255, 255, .12);
    border-radius: 12px;
    color: #fff;
    background: rgba(8, 10, 14, .78);
    backdrop-filter: blur(12px);
    pointer-events: none;
  }
  .toast-copy { min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .toast-copy small { color: rgba(255, 255, 255, .62); font-size: 9px; font-weight: 700; letter-spacing: .08em; text-transform: uppercase; }
  .toast-copy strong { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12px; }
  .player-error {
    position: absolute;
    z-index: 20;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: safe center;
    padding: clamp(20px, 5vw, 56px);
    margin: 0;
    border-radius: inherit;
    color: #fff;
    background: radial-gradient(ellipse at 50% 42%, rgba(39, 28, 34, .97), rgba(7, 9, 13, .99) 72%);
    text-align: center;
    overflow-y: auto;
  }
  .has-overlay video { visibility: hidden; }
  .has-overlay :global(.shaka-controls-container) { display: none; }
  .error-mark { display: grid; place-items: center; width: 42px; height: 42px; margin-bottom: 18px; border: 1px solid rgba(248, 113, 113, .3); border-radius: 50%; color: #fca5a5; background: rgba(239, 68, 68, .12); font-size: 20px; font-weight: 700; }
  .error-eyebrow { margin: 0 0 8px; color: #fca5a5; font-size: 11px; font-weight: 750; letter-spacing: .12em; text-transform: uppercase; }
  .player-error h2 { max-width: 100%; margin: 0; font-size: clamp(20px, 3vw, 28px); line-height: 1.2; letter-spacing: -.025em; }
  .error-detail { max-width: min(100%, 560px); margin: 12px 0 0; color: #a7afbd; font-size: 13px; line-height: 1.55; overflow-wrap: anywhere; }
  .error-actions { display: flex; flex-wrap: wrap; justify-content: center; gap: 10px; margin-top: 24px; }
  .error-actions button, .error-actions a { display: inline-flex; align-items: center; justify-content: center; min-height: 40px; padding: 0 17px; border: 1px solid var(--border); border-radius: 10px; color: #e8ebf1; background: rgba(255, 255, 255, .045); text-decoration: none; font: inherit; font-size: 12px; font-weight: 700; cursor: pointer; }
  .error-actions .retry-button { border-color: color-mix(in srgb, var(--accent) 65%, transparent); color: #fff; background: var(--accent); }
  .error-actions button:hover, .error-actions a:hover { filter: brightness(1.12); }
  .error-actions button:focus-visible, .error-actions a:focus-visible { outline: 2px solid #fff; outline-offset: 3px; }
  .badge.premium { color: #f6c667; border-color: rgba(246, 198, 103, .34); }
  @media (prefers-reduced-motion: no-preference) {
    .player-error { animation: error-in .18s ease-out both; }
    .program-toast { animation: toast-in .2s ease-out both; }
    @keyframes error-in { from { opacity: 0; } to { opacity: 1; } }
    @keyframes toast-in { from { opacity: 0; transform: translateY(6px); } to { opacity: 1; transform: translateY(0); } }
  }
  @keyframes spin { to { transform: rotate(360deg); } }
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
    .stage { width: min(100%, calc((100dvh - 100px) * 16 / 9)); margin-inline: auto; }
  }
  @media (max-width: 420px) {
    .player-error { padding: 12px; }
    .error-detail { font-size: 11px; }
  }

  @media (max-width: 640px) {
    .layout { gap: 14px; }
    .stage { border-radius: 12px; box-shadow: 0 14px 36px rgba(0, 0, 0, .22); }
    .live-pill { padding: 2px 6px; font-size: 9px; }
    .player-controls { padding: 0 10px 9px; }
    .player-meta { gap: 2px; max-width: 82%; margin-bottom: 9px; }
    .player-meta-top { gap: 6px; }
    .player-meta-top > strong { font-size: 8.5px; }
    .player-program { font-size: 13px; }
    .player-program-description { font-size: 8px; line-height: 1.3; }
    .player-seek { height: 2px; margin-bottom: 6px; }
    .player-control-row { min-height: 28px; gap: 8px; }
    .player-control-left, .player-control-right { gap: 2px; }
    .icon-control { width: 28px; min-width: 28px; height: 28px; border-radius: 7px; }
    .icon-control svg { width: 14px; height: 14px; }
    .quality-control { height: 24px; min-width: 30px; padding: 0 6px; border-radius: 7px; font-size: 8px; }
    .player-live-clock { margin-left: 1px; font-size: 7.5px; }
    .control-menu { min-width: 132px; bottom: calc(100% + 5px); }
    .control-menu button { min-height: 30px; font-size: 10px; }
    .program-toast { left: 10px; bottom: 92px; max-width: calc(100% - 20px); }
    .error-detail { font-size: 12px; }
    .error-actions { margin-top: 18px; }
    aside { gap: 16px; padding-top: 0; }
    .channel-card { padding: 8px; }
    .channel-card img { width: 52px; height: 38px; }
    h1 { font-size: 18px; }
    .quality-options button { padding: 7px 3px; font-size: 10.5px; }
    .guide li { grid-template-columns: 56px minmax(0, 1fr); padding: 8px; }
    .desc { -webkit-line-clamp: 3; }
  }
</style>
