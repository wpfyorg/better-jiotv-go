<script>
  import { untrack } from "svelte";
  import { api, formatTime } from "../lib/api.js";

  let { id } = $props();
  let season = $state(0);
  let episodes = $state([]);
  let loading = $state(true);
  let error = $state("");

  $effect(() => {
    const showID = id;
    const s = season;
    untrack(async () => {
      loading = true;
      error = "";
      try {
        const d = await api(`/api/ott/show/${encodeURIComponent(showID)}${s ? "?season=" + s : ""}`);
        episodes = (d.episodes ?? []).sort((a, b) => a.episodeNo - b.episodeNo);
      } catch (err) {
        error = err.message;
        episodes = [];
      } finally {
        loading = false;
      }
    });
  });

  const title = $derived(episodes[0]?.showName || "Episodes");
  const current = $derived(season || episodes[0]?.season || 1);

  function minutes(sec) {
    return sec ? Math.round(sec / 60) + " min" : "";
  }
</script>

<div class="head">
  <a class="btn" href="#/ott">← On demand</a>
  <h1>{title}</h1>
  <label>
    Season
    <select class="input" value={current} onchange={(e) => (season = Number(e.currentTarget.value))}>
      {#each Array.from({ length: Math.max(10, current) }, (_, i) => i + 1) as n}
        <option value={n}>{n}</option>
      {/each}
    </select>
  </label>
</div>

{#if loading}
  <p class="muted">Loading…</p>
{:else if error}
  <p class="error" role="alert">{error}</p>
{:else if episodes.length === 0}
  <p class="muted">No episodes in season {current}.</p>
{:else}
  <ol class="episodes">
    {#each episodes as ep (ep.contentId)}
      <li>
        <a href={"#/play/" + ep.contentId}>
          <span class="art"><img src={ep.thumbnail} alt="" loading="lazy" /></span>
          <span class="text">
            <strong>{ep.episodeNo ? `E${ep.episodeNo} · ` : ""}{ep.name}</strong>
            <span class="muted">{minutes(ep.totalDuration)}</span>
            {#if ep.description}<span class="desc muted">{ep.description}</span>{/if}
          </span>
        </a>
      </li>
    {/each}
  </ol>
{/if}

<style>
  .head { display: flex; gap: 14px; align-items: center; flex-wrap: wrap; margin-bottom: 16px; }
  h1 { font-size: 22px; margin: 0; flex: 1; }
  label { display: flex; gap: 8px; align-items: center; color: var(--muted); }
  label .input { width: auto; }
  .episodes { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 10px; }
  .episodes a { display: grid; grid-template-columns: 200px 1fr; gap: 14px; text-decoration: none; padding: 8px; border-radius: var(--radius); border: 1px solid transparent; }
  .episodes a:hover, .episodes a:focus-visible { border-color: var(--border); background: var(--surface); }
  @media (max-width: 600px) { .episodes a { grid-template-columns: 120px 1fr; } }
  .art { aspect-ratio: 16 / 9; border-radius: 8px; overflow: hidden; background: #1d2230; }
  .art img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .text { display: flex; flex-direction: column; gap: 3px; }
  .desc { font-size: 13px; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
</style>
