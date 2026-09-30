<script>
  import { onMount } from "svelte";
  import { api, loadChannels, looksLikeUnlockCode } from "../lib/api.js";

  const saved = (() => {
    try {
      return JSON.parse(localStorage.getItem("channelFilters") || "{}");
    } catch {
      return {};
    }
  })();

  let channels = $state([]);
  let error = $state("");
  let loading = $state(true);
  let query = $state("");
  let category = $state(saved.category ?? "");
  let language = $state(saved.language ?? "");
  let extrasOnly = $state(saved.extrasOnly ?? false);
  let hdOnly = $state(saved.hdOnly ?? false);
  let showUnplayable = $state(saved.showUnplayable ?? false);

  $effect(() => {
    try {
      localStorage.setItem("channelFilters", JSON.stringify({ category, language, extrasOnly, hdOnly, showUnplayable }));
    } catch {}
  });

  // If (and only if) the search box holds something shaped like the extras
  // unlock code, try it against the server. An ordinary search never
  // matches this shape, so it never leaves the browser.
  let triedCode = "";
  $effect(() => {
    const q = query.trim();
    if (q && q !== triedCode && looksLikeUnlockCode(q)) {
      triedCode = q;
      api("/api/extras/unlock", { method: "POST", body: { code: q } })
        .then(() => window.dispatchEvent(new CustomEvent("jiotv:extras-changed")))
        .catch(() => {});
    }
  });

  const categories = $derived([...new Set(channels.map((c) => c.category).filter(Boolean))].sort());
  const languages = $derived([...new Set(channels.map((c) => c.language).filter(Boolean))].sort());

  const visible = $derived.by(() => {
    const q = query.trim().toLowerCase();
    return channels.filter(
      (c) =>
        (showUnplayable || c.playable) &&
        (!extrasOnly || c.extras) &&
        (!hdOnly || c.hd) &&
        (!category || c.category === category) &&
        (!language || c.language === language) &&
        (!q || c.name.toLowerCase().includes(q)),
    );
  });

  onMount(async () => {
    try {
      channels = await loadChannels();
    } catch (err) {
      error = err.message;
    } finally {
      loading = false;
    }
  });
</script>

<section class="filters" aria-label="Filters">
  <input class="input search" type="search" placeholder="Search channels" bind:value={query} aria-label="Search channels" />
  <select class="input" bind:value={category} aria-label="Category">
    <option value="">All categories</option>
    {#each categories as c}<option>{c}</option>{/each}
  </select>
  <select class="input" bind:value={language} aria-label="Language">
    <option value="">All languages</option>
    {#each languages as l}<option>{l}</option>{/each}
  </select>
  <label class="toggle"><input type="checkbox" bind:checked={hdOnly} /> HD</label>
  <label class="toggle"><input type="checkbox" bind:checked={extrasOnly} /> Extra channels only</label>
  <label class="toggle"><input type="checkbox" bind:checked={showUnplayable} /> Show unavailable</label>
</section>

{#if loading}
  <p class="muted">Loading channels…</p>
{:else if error}
  <p class="error">Couldn't load channels: {error}</p>
{:else}
  <p class="muted count">{visible.length} of {channels.length} channels</p>
  <ul class="grid">
    {#each visible as c (c.id)}
      <li>
        <a class="tile" class:off={!c.playable} href={"#/watch/" + encodeURIComponent(c.id)} title={c.playable ? c.name : c.name + " (needs a JioTV login)"}>
          <span class="logo"><img src={c.logo} alt="" loading="lazy" decoding="async" /></span>
          <span class="name">{c.name}</span>
          <span class="tags">
            {#if c.hd}<span class="badge">HD</span>{/if}
            {#if c.extras}<span class="badge extras">Extra</span>{/if}
            {#if c.requiresSubscription}<span class="badge subscription">Subscription required</span>{/if}
          </span>
        </a>
      </li>
    {/each}
  </ul>
  {#if visible.length === 0}<p class="muted">No channels match these filters.</p>{/if}
{/if}

<style>
  .filters { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; margin-bottom: 12px; }
  .filters .input { width: auto; }
  .search { flex: 1 1 240px; }
  .toggle { display: inline-flex; gap: 6px; align-items: center; color: var(--muted); padding: 0 6px; }
  .count { margin: 4px 0 12px; font-size: 13px; }
  .grid {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 12px;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
  }
  .tile {
    display: flex;
    flex-direction: column;
    gap: 8px;
    height: 100%;
    padding: 12px;
    border-radius: var(--radius);
    background: var(--surface);
    border: 1px solid var(--border);
    text-decoration: none;
    transition: transform 0.12s ease, border-color 0.12s ease;
  }
  .tile:hover, .tile:focus-visible { transform: translateY(-2px); border-color: var(--accent); }
  .tile.off { opacity: 0.45; }
  /* Many logos are white on transparent, so keep the tile dark in both themes. */
  .logo { aspect-ratio: 16 / 10; display: grid; place-items: center; background: #1d2230; border-radius: 8px; overflow: hidden; }
  .logo img { max-width: 80%; max-height: 80%; object-fit: contain; }
  .name { font-size: 13px; font-weight: 600; line-height: 1.3; }
  .tags { display: flex; gap: 4px; margin-top: auto; }
  .badge.subscription { color: var(--danger); border-color: color-mix(in srgb, var(--danger) 45%, transparent); }
</style>
