<script>
  import { onDestroy } from "svelte";
  import { api, loadChannels, formatTime } from "../lib/api.js";
  import { loadScript } from "../lib/loadScript.js";

  let { id } = $props();

  let quality = $state(localStorage.getItem("quality") || "auto");
  let channel = $state(null);
  let guide = $state([]);
  let guideError = $state("");
  let video = $state();
  let playerError = $state("");
  let cleanup = null;

  const hevc = 'video/mp4; codecs="hev1.1.6.L120.90"';

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

  // Mirrors VodPlayer.svelte: Shaka for DASH + Widevine (license through
  // /live/key/:id, reached with the same key prefix or admin session as
  // everything else here), hls.js 1.7.3 for HLS (it plays HEVC in MPEG-TS,
  // which newer hls.js releases and Chrome's native HLS cannot), and a plain
  // <video> src as the last resort when neither can handle the stream.
  async function start(channelID, q) {
    cleanup?.();
    cleanup = null;
    playerError = "";
    try {
      const d = await api(`/api/live/play/${encodeURIComponent(channelID)}?q=${q}`);
      if (d.dash) {
        await loadScript("/static/external/shaka-player.ui.js");
        const shaka = window.shaka;
        shaka.polyfill.installAll();
        const player = new shaka.Player();
        await player.attach(video);
        if (d.license) {
          player.configure({
            drm: {
              servers: { "com.widevine.alpha": d.license },
              advanced: { "com.widevine.alpha": { videoRobustness: "SW_SECURE_CRYPTO", audioRobustness: "SW_SECURE_CRYPTO" } },
            },
          });
        }
        player.addEventListener("error", (e) => (playerError = "Playback error " + (e.detail?.code ?? "")));
        cleanup = () => player.destroy();
        await player.load(d.url);
      } else {
        await loadScript("/static/external/hls-1.7.3.min.js");
        const Hls = window.Hls;
        if (Hls.isSupported() && (MediaSource.isTypeSupported(hevc) || !/H_265|hevc/i.test(d.url))) {
          const hls = new Hls({ capLevelToPlayerSize: false });
          hls.on(Hls.Events.ERROR, (_, data) => {
            if (data.fatal) playerError = "Playback error: " + data.details;
          });
          hls.loadSource(d.url);
          hls.attachMedia(video);
          cleanup = () => hls.destroy();
        } else if (video.canPlayType("application/vnd.apple.mpegurl")) {
          video.src = d.url;
          cleanup = () => video.removeAttribute("src");
        } else {
          throw new Error("This channel is HEVC (H.265), which this browser can't play. Try Chrome, Edge or Safari.");
        }
      }
      await video.play().catch(() => {});
    } catch (err) {
      playerError = err.message || String(err);
    }
  }

  $effect(() => {
    if (video) start(id, quality);
  });

  onDestroy(() => cleanup?.());
</script>

<div class="layout">
  <div class="stage">
    <!-- svelte-ignore a11y_media_has_caption -->
    <video bind:this={video} controls autoplay playsinline></video>
    {#if playerError}<p class="error" role="alert">{playerError}</p>{/if}
  </div>

  <aside>
    <div class="head">
      {#if channel}<img src={channel.logo} alt="" />{/if}
      <div>
        <h1>{channel?.name ?? id}</h1>
        <p class="muted">
          {[channel?.category, channel?.language].filter(Boolean).join(" · ")}
          {#if channel?.tvplus}<span class="badge tvplus">TV+</span>{/if}
        </p>
      </div>
    </div>

    <label class="quality">
      Quality
      <select class="input" bind:value={quality}>
        <option value="auto">Auto</option>
        <option value="high">High</option>
        <option value="medium">Medium</option>
        <option value="low">Low</option>
      </select>
    </label>

    <h2>Guide</h2>
    {#if guideError}
      <p class="muted">No guide for this channel.</p>
    {:else if guide.length === 0}
      <p class="muted">Loading…</p>
    {:else}
      <ol class="guide">
        {#each guide as p, i}
          <li class:now={i === 0 && p.startEpoch <= Date.now()}>
            <span class="time">{formatTime(p.startEpoch)}</span>
            <span>
              <strong>{p.showname}</strong>
              {#if i === 0 && p.description}<span class="desc muted">{p.description}</span>{/if}
            </span>
          </li>
        {/each}
      </ol>
    {/if}
    <a class="btn" href="#/">← All channels</a>
  </aside>
</div>

<style>
  .layout { display: grid; gap: 20px; grid-template-columns: minmax(0, 1fr) 320px; align-items: start; }
  @media (max-width: 900px) { .layout { grid-template-columns: 1fr; } }
  .stage { aspect-ratio: 16 / 9; background: #000; border-radius: var(--radius); overflow: hidden; }
  video { width: 100%; height: 100%; display: block; background: #000; }
  .error { margin: 8px 0 0; color: var(--error, #f66); }
  aside { display: flex; flex-direction: column; gap: 14px; }
  .head { display: flex; gap: 12px; align-items: center; }
  .head img { width: 64px; height: 40px; object-fit: contain; background: #1d2230; border-radius: 8px; padding: 4px; }
  h1 { font-size: 20px; margin: 0; }
  h1 + p { margin: 2px 0 0; font-size: 13px; }
  h2 { font-size: 14px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); margin: 6px 0 0; }
  .quality { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .quality .input { width: auto; }
  .guide { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  .guide li { display: grid; grid-template-columns: 56px 1fr; gap: 8px; padding: 8px; border-radius: 8px; font-size: 14px; }
  .guide li.now { background: var(--surface-2); }
  .time { color: var(--muted); font-variant-numeric: tabular-nums; }
  .desc { display: block; font-size: 13px; margin-top: 2px; }
</style>
