<script>
  import { onDestroy } from "svelte";
  import { api } from "../lib/api.js";
  import { loadScript } from "../lib/loadScript.js";

  let { id } = $props();
  let video = $state();
  let info = $state(null);
  let error = $state("");
  let cleanup = null;

  const hevc = 'video/mp4; codecs="hev1.1.6.L120.90"';

  async function start(contentID) {
    cleanup?.();
    cleanup = null;
    error = "";
    info = null;
    try {
      const d = await api("/api/ott/play/" + encodeURIComponent(contentID));
      info = d;
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
        player.addEventListener("error", (e) => (error = "Playback error " + (e.detail?.code ?? "")));
        cleanup = () => player.destroy();
        await player.load(d.url);
      } else {
        await loadScript("/static/external/hls-1.7.3.min.js");
        const Hls = window.Hls;
        if (Hls.isSupported() && (MediaSource.isTypeSupported(hevc) || !/H_265|hevc/i.test(d.url))) {
          const hls = new Hls({ capLevelToPlayerSize: false });
          hls.on(Hls.Events.ERROR, (_, data) => {
            if (data.fatal) error = "Playback error: " + data.details;
          });
          hls.loadSource(d.url);
          hls.attachMedia(video);
          cleanup = () => hls.destroy();
        } else if (video.canPlayType("application/vnd.apple.mpegurl")) {
          video.src = d.url;
          cleanup = () => video.removeAttribute("src");
        } else {
          throw new Error("This title is HEVC (H.265), which this browser can't play. Try Chrome, Edge or Safari.");
        }
      }
      await video.play().catch(() => {});
    } catch (err) {
      error = err.message || String(err);
    }
  }

  $effect(() => {
    if (video) start(id);
  });

  onDestroy(() => cleanup?.());
</script>

<div class="wrap">
  <div class="stage">
    <!-- svelte-ignore a11y_media_has_caption -->
    <video bind:this={video} controls autoplay playsinline></video>
  </div>
  <div class="info">
    <div>
      <h1>{info?.name ?? "Loading…"}</h1>
      {#if info}<p class="muted">{info.provider}{info.duration ? " · " + Math.round(info.duration / 60) + " min" : ""}</p>{/if}
    </div>
    <button class="btn" onclick={() => history.back()}>← Back</button>
  </div>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</div>

<style>
  .wrap { max-width: 1200px; margin: 0 auto; }
  .stage { aspect-ratio: 16 / 9; background: #000; border-radius: var(--radius); overflow: hidden; }
  video { width: 100%; height: 100%; display: block; background: #000; }
  .info { display: flex; justify-content: space-between; align-items: center; gap: 16px; margin-top: 14px; }
  h1 { font-size: 20px; margin: 0; }
  p { margin: 2px 0 0; }
</style>
