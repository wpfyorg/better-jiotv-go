<script>
  import { onDestroy, onMount } from "svelte";
  import { api, keyBase, loadChannels, formatTime } from "../lib/api.js";
  import { createShakaPlayer, isDrmPlaybackError, playWithAutoplay, widevineCapability } from "../lib/shakaPlayer.js";

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
  let cleanup = null;
  let runID = 0;
  let activityTimer = null;
  let toastTimer = null;
  let lastProgram = "";

  const fullStageKinds = new Set(["offline", "playback", "no-stream", "protected", "subscription", "restricted", "extras-signin", "service"]);

  let currentProgram = $derived(guide.find((program) => isNow(program, clock)) ?? null);

  function gated(path) {
    return keyBase ? keyBase + path.replace(/^\//, "") : path;
  }

  function hlsFallback(channelID, q) {
    return gated(`/live/${encodeURIComponent(q)}/${encodeURIComponent(channelID)}.m3u8`);
  }

  function isNow(program, now = Date.now()) {
    return program?.startEpoch <= now && program?.endEpoch > now;
  }

  function setPlayerState(kind, eyebrow, title, detail = "") {
    playerState = { kind, eyebrow, title, detail };
  }

  function showPlaybackError(err, drmCapability = null) {
    const message = String(err?.message || "").toLowerCase();
    const status = Number(err?.status);

    if (!navigator.onLine) {
      setPlayerState("offline", "Connection issue", "You’re offline", "Reconnect to the internet, then try the stream again.");
    } else if (isDrmPlaybackError(err) && drmCapability && !drmCapability.usable) {
      setPlayerState("protected", "Protected playback", "Protected playback unavailable", "This browser cannot play the protected stream. Try a supported browser or device, then retry.");
    } else if (message.includes("extras is not connected") || message.includes("not logged in")) {
      setPlayerState("extras-signin", "Extras", "Connect extras to play", "This channel comes from extras. Connect the extras account, then return here to start playback.");
    } else if (message.includes("not in your extras plan") || message.includes("not subscribed")) {
      setPlayerState("subscription", "Account access", "Subscription required", "This channel is not included with the current account. Choose another channel or retry after the account has access.");
    } else if (status === 404 && message.includes("no stream")) {
      setPlayerState("no-stream", "Live TV", "No live stream available", "This channel does not have a playable live stream right now.");
    } else if (status === 404 && message.includes("active account")) {
      setPlayerState("restricted", "Account access", "Not available on this account", "Playback was refused for this account. Access may depend on the active plan or provider entitlement.");
    } else if (status === 403) {
      setPlayerState("subscription", "Account access", "Subscription required", "This channel is not included with the current account. Choose another channel or retry after the account has access.");
    } else if (status >= 500) {
      setPlayerState("service", "Live TV", "Service temporarily unavailable", "The provider could not start this live stream. Wait a moment and try again.");
    } else {
      setPlayerState("playback", "Playback issue", "Playback unavailable", "The live stream could not be started. Check the connection or try again in a moment.");
    }
  }

  function markActivity() {
    showMeta = true;
    clearTimeout(activityTimer);
    activityTimer = setTimeout(() => (showMeta = false), 3000);
  }

  async function tapToPlay() {
    if (!video) return;
    video.muted = false;
    try {
      await video.play();
      setPlayerState("playing", "", "", "");
      markActivity();
    } catch {
      showPlaybackError(new Error("playback failed"));
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
    lastProgram = "";
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

  $effect(() => {
    const title = currentProgram?.showname || "";
    if (!title) return;
    if (lastProgram && lastProgram !== title) {
      programToast = title;
      clearTimeout(toastTimer);
      toastTimer = setTimeout(() => (programToast = ""), 4200);
    }
    lastProgram = title;
  });

  async function start(channelID, q) {
    const thisRun = ++runID;
    await cleanup?.();
    cleanup = null;
    setPlayerState("loading", "Live TV", "Starting live TV…", "");
    markActivity();

    try {
      const status = await api("/api/status").catch(() => null);
      if (thisRun !== runID) return;
      if (channelID.startsWith("ex_") && status?.extras?.enabled && !status?.extras?.connected) {
        setPlayerState("extras-signin", "Extras", "Connect extras to play", "This channel comes from extras. Connect the extras account, then return here to start playback.");
        return;
      }

      const d = await api(`/api/live/play/${encodeURIComponent(channelID)}?q=${q}`);
      if (thisRun !== runID) return;
      const session = await createShakaPlayer(playerContainer, video);
      const player = session.player;
      const onPlaying = () => {
        if (thisRun === runID) setPlayerState("playing", "", "", "");
      };
      const onWaiting = () => {
        if (thisRun === runID && !fullStageKinds.has(playerState.kind)) setPlayerState("buffering", "Live TV", "Buffering…", "");
      };
      const onStalled = () => {
        if (thisRun === runID && !fullStageKinds.has(playerState.kind)) setPlayerState("reconnecting", "Live TV", "Reconnecting live stream…", "");
      };
      video.addEventListener("playing", onPlaying);
      video.addEventListener("waiting", onWaiting);
      video.addEventListener("stalled", onStalled);
      cleanup = async () => {
        video?.removeEventListener("playing", onPlaying);
        video?.removeEventListener("waiting", onWaiting);
        video?.removeEventListener("stalled", onStalled);
        await session.destroy().catch(() => {});
      };
      const drmCapability = d.dash && d.license ? await widevineCapability() : null;

      if (d.license) {
        player.configure({
          drm: {
            servers: { "com.widevine.alpha": d.license },
            advanced: { "com.widevine.alpha": { videoRobustness: "SW_SECURE_CRYPTO", audioRobustness: "SW_SECURE_CRYPTO" } },
          },
          streaming: { bufferBehind: 2, bufferingGoal: 6, rebufferingGoal: 2 },
        });
      }

      let fallingBack = false;
      const fallbackToHls = async () => {
        if (fallingBack) return;
        fallingBack = true;
        setPlayerState("reconnecting", "Live TV", "Reconnecting live stream…", "");
        await player.unload();
        await player.load(hlsFallback(channelID, q));
        const played = await playWithAutoplay(video, { allowMutedFallback: false });
        if (thisRun !== runID) return;
        setPlayerState(played ? "playing" : "tap", "", played ? "" : "Tap to play", "");
      };

      player.addEventListener("error", (event) => {
        if (thisRun !== runID) return;
        const detail = event.detail;
        if (d.dash && d.license && isDrmPlaybackError(detail)) {
          fallbackToHls().catch(() => showPlaybackError(detail, drmCapability));
        } else {
          showPlaybackError(detail, drmCapability);
        }
      });

      player.addEventListener("buffering", (event) => {
        if (thisRun !== runID || fullStageKinds.has(playerState.kind)) return;
        const buffering = event?.buffering ?? event?.detail?.buffering;
        setPlayerState(buffering ? "buffering" : "playing", "Live TV", buffering ? "Buffering…" : "", "");
      });

      try {
        await player.load(d.url);
      } catch (err) {
        if (d.dash && d.license && isDrmPlaybackError(err)) {
          try {
            await fallbackToHls();
          } catch {
            showPlaybackError(err, drmCapability);
          }
          return;
        }
        throw err;
      }
      const played = await playWithAutoplay(video, { allowMutedFallback: false });
      if (thisRun !== runID) return;
      setPlayerState(played ? "playing" : "tap", "", played ? "" : "Tap to play", "");
    } catch (err) {
      if (thisRun === runID) showPlaybackError(err);
    }
  }

  $effect(() => {
    if (playerContainer && video) start(id, quality);
  });

  onMount(() => {
    const clockTimer = setInterval(() => (clock = Date.now()), 15000);
    const onOffline = () => setPlayerState("offline", "Connection issue", "You’re offline", "Reconnect to the internet, then try the stream again.");
    const onOnline = () => {
      setPlayerState("reconnecting", "Live TV", "Reconnecting live stream…", "");
      start(id, quality);
    };
    window.addEventListener("offline", onOffline);
    window.addEventListener("online", onOnline);
    markActivity();
    return () => {
      clearInterval(clockTimer);
      window.removeEventListener("offline", onOffline);
      window.removeEventListener("online", onOnline);
    };
  });

  onDestroy(() => {
    runID += 1;
    clearTimeout(activityTimer);
    clearTimeout(toastTimer);
    cleanup?.();
  });
</script>

<div class="layout">
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div
    class:has-overlay={fullStageKinds.has(playerState.kind)}
    class="stage"
    role="region"
    aria-label="Live player"
    bind:this={playerContainer}
    onpointermove={markActivity}
    onpointerdown={markActivity}
    onkeydown={markActivity}
    onfocusin={markActivity}
  >
    <!-- svelte-ignore a11y_media_has_caption -->
    <video bind:this={video} autoplay playsinline></video>
    {#if showMeta && !fullStageKinds.has(playerState.kind)}
      <div class="player-meta" aria-hidden="true">
        <span class="live-pill"><span></span>LIVE</span>
        <div class="player-copy">
          <strong>{channel?.name ?? id}</strong>
          {#if currentProgram}<small>{currentProgram.showname}</small>{/if}
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
          {#if playerState.kind === "extras-signin"}
            <a class="retry-button" href="#/settings">Open account settings</a>
          {:else}
            <button class="retry-button" onclick={() => start(id, quality)}>Try again</button>
          {/if}
          {#if playerState.kind !== "service"}<a class="channels-button" href="#/">All channels</a>{/if}
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
          {#if channel?.premium}<span class="badge premium">Premium</span>{/if}
          {#if channel?.extras}<span class="badge extras">Extra</span>{/if}
        </p>
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
    justify-content: center;
    padding: clamp(20px, 5vw, 56px);
    margin: 0;
    border-radius: inherit;
    color: #fff;
    background: radial-gradient(ellipse at 50% 42%, rgba(39, 28, 34, .97), rgba(7, 9, 13, .99) 72%);
    text-align: center;
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

  @media (max-width: 640px) {
    .layout { gap: 14px; }
    .stage { border-radius: 12px; box-shadow: 0 14px 36px rgba(0, 0, 0, .22); }
    .player-meta { top: 10px; left: 10px; max-width: calc(100% - 20px); padding: 6px 8px; border-radius: 9px; }
    .live-pill { padding: 2px 6px; font-size: 9px; }
    .player-copy strong { font-size: 12px; }
    .player-copy small { display: none; }
    .program-toast { left: 10px; bottom: 56px; max-width: calc(100% - 20px); }
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
