<script>
  import { api, loadChannels, formatTime } from "../lib/api.js";

  let { id } = $props();

  let quality = $state(localStorage.getItem("quality") || "auto");
  let channel = $state(null);
  let guide = $state([]);
  let guideError = $state("");

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

  const playerSrc = $derived(`/mpd/${encodeURIComponent(id)}?q=${quality}`);
</script>

<div class="layout">
  <div class="stage">
    {#key playerSrc}
      <iframe
        src={playerSrc}
        title={channel?.name ?? "Player"}
        allow="autoplay; fullscreen; encrypted-media; picture-in-picture"
        allowfullscreen
      ></iframe>
    {/key}
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
  iframe { width: 100%; height: 100%; border: 0; display: block; }
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
