<script>
  import { onDestroy } from "svelte";
  import { api, keyBase } from "../lib/api.js";
  import { createShakaPlayer, playbackErrorMessage, playWithAutoplay, widevineCapability } from "../lib/shakaPlayer.js";

  let { id } = $props();
  let playerContainer = $state();
  let video = $state();
  let info = $state(null);
  let error = $state("");
  let cleanup = null;

  function gated(path) {
    return keyBase ? keyBase + path.replace(/^\//, "") : path;
  }

  async function start(contentID) {
    cleanup?.();
    cleanup = null;
    error = "";
    info = null;

    try {
      const d = await api("/api/ott/play/" + encodeURIComponent(contentID));
      info = d;
      const session = await createShakaPlayer(playerContainer, video);
      const player = session.player;
      cleanup = () => session.destroy().catch(() => {});
      const drmCapability = d.license ? await widevineCapability() : null;

      if (d.license) {
        player.configure({
          drm: {
            servers: { "com.widevine.alpha": gated(d.license) },
            advanced: { "com.widevine.alpha": { videoRobustness: "SW_SECURE_CRYPTO", audioRobustness: "SW_SECURE_CRYPTO" } },
          },
          streaming: { bufferBehind: 2, bufferingGoal: 6, rebufferingGoal: 2 },
        });
      }

      player.addEventListener("error", (event) => (error = playbackErrorMessage(event.detail, drmCapability)));
      await player.load(d.url);
      await playWithAutoplay(video);
    } catch (err) {
      const capability = info?.license ? await widevineCapability().catch(() => null) : null;
      error = playbackErrorMessage(err, capability) || err.message || String(err);
    }
  }

  $effect(() => {
    if (playerContainer && video) start(id);
  });

  onDestroy(() => cleanup?.());
</script>

<div class="wrap">
  <div class="stage" bind:this={playerContainer}>
    <!-- svelte-ignore a11y_media_has_caption -->
    <video bind:this={video} autoplay playsinline></video>
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
  .info { display: flex; justify-content: space-between; align-items: center; gap: 16px; margin-top: 16px; }
  h1 { font-size: 21px; line-height: 1.25; letter-spacing: -.02em; margin: 0; }
  p { margin: 4px 0 0; font-size: 13px; }
  .error { margin-top: 12px; padding: 9px 12px; border: 1px solid color-mix(in srgb, var(--danger) 38%, transparent); border-radius: 10px; background: color-mix(in srgb, var(--danger) 7%, var(--surface)); }

  @media (max-width: 640px) {
    .stage { border-radius: 12px; box-shadow: 0 14px 36px rgba(0, 0, 0, .22); }
    .info { align-items: flex-start; gap: 10px; margin-top: 12px; }
    h1 { font-size: 18px; }
    .info .btn { padding: 7px 10px; font-size: 12px; }
  }
</style>
