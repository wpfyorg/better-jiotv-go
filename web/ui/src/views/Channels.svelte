<script>
  import { onMount } from "svelte";
  import { api, loadChannels, looksLikeUnlockCode } from "../lib/api.js";
  import { latestOnly } from "../lib/latest.js";

  const saved = (() => {
    try {
      return JSON.parse(localStorage.getItem("channelFilters") || "{}");
    } catch {
      return {};
    }
  })();

  let channels = $state([]);
  // When every listed channel comes from extras the pill tells the viewer nothing.
  const mixedSources = $derived(channels.some((c) => c.extras) && channels.some((c) => !c.extras));
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
  // The initial load and the reload after an unlock can overlap; only the one
  // started last may publish, so a late pre-unlock result cannot overwrite the
  // extras catalogue (or restore an error the reload cleared).
  const loads = latestOnly();
  let triedCode = "";
  // Set once a code is accepted. A wrong code stays silent on purpose: it must
  // not reveal that this box does anything other than search.
  let unlockNotice = $state(null);
  $effect(() => {
    const q = query.trim();
    // A rejected or failed attempt is remembered only while the box still holds
    // that value, so it is not resent on every edit but can be tried again once
    // the viewer has cleared or changed it.
    if (triedCode && q !== triedCode) triedCode = "";
    if (q && q !== triedCode && looksLikeUnlockCode(q)) {
      triedCode = q;
      api("/api/extras/unlock", { method: "POST", body: { code: q } })
        .then(async (d) => {
          // The code is not a search term; clear it so the list is not empty,
          // unless the viewer has meanwhile typed something else.
          if (query.trim() === q) query = "";
          triedCode = "";
          unlockNotice = { connected: !!d?.extras?.connected };
          window.dispatchEvent(new CustomEvent("jiotv:extras-changed"));
          const isLatest = loads.start();
          try {
            const list = await loadChannels(true);
            if (isLatest()) {
              channels = list;
              // A failed first load must not keep hiding a list that now loaded,
              // and this refresh may finish before the older request does.
              error = "";
              loading = false;
            }
          } catch (err) {
            // The code was accepted but the list could not be refreshed: keep what
            // is shown and say so, instead of claiming extra channels are listed.
            if (isLatest()) {
              if (unlockNotice) unlockNotice = { ...unlockNotice, refreshFailed: true };
              // The older initial request can no longer publish, so end the loading
              // state here; with nothing to show, report the failure instead.
              if (!channels.length && !error) error = err.message;
              loading = false;
            }
          }
        })
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
    const isLatest = loads.start();
    try {
      const list = await loadChannels();
      if (isLatest()) channels = list;
    } catch (err) {
      if (isLatest()) error = err.message;
    } finally {
      // An unlock refresh that started meanwhile owns the loading state.
      if (isLatest()) loading = false;
    }
  });
</script>

{#if unlockNotice}
  <div class="unlock-notice" role="status">
    <span class="unlock-icon" aria-hidden="true">✓</span>
    <div class="unlock-copy">
      <strong>Extra channels unlocked</strong>
      <span>
        {#if unlockNotice.refreshFailed}
          The channel list could not be refreshed. Reload the page to see the extra channels.
        {:else if unlockNotice.connected}
          The extra source is connected; its channels are listed below.
        {:else}
          Login with number with access to the extra in Settings to be able to play its channels.
        {/if}
      </span>
    </div>
    {#if !unlockNotice.connected}<a class="unlock-link" href="#/settings">Open Settings</a>{/if}
    <button class="unlock-dismiss" aria-label="Dismiss" onclick={() => (unlockNotice = null)}>×</button>
  </div>
{/if}

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
          <span class="logo">
            <img src={c.logo} alt="" loading="lazy" decoding="async" />
            {#if (mixedSources && c.extras) || c.requiresSubscription}
              <span class="flags">
                {#if mixedSources && c.extras}<span class="flag flag-extras">Extra</span>{/if}
                {#if c.requiresSubscription}<span class="flag flag-premium">Premium</span>{/if}
              </span>
            {/if}
          </span>
          <span class="name">{c.name}</span>
          <span class="tags">{#if c.hd}<span class="badge">HD</span>{/if}</span>
        </a>
      </li>
    {/each}
  </ul>
  {#if visible.length === 0}<p class="muted">No channels match these filters.</p>{/if}
{/if}

<style>
  .unlock-notice {
    display: flex;
    align-items: center;
    gap: 12px;
    margin: 0 0 14px;
    padding: 10px 12px;
    border: 1px solid color-mix(in srgb, var(--extras) 45%, transparent);
    border-radius: 12px;
    background: color-mix(in srgb, var(--extras) 12%, var(--surface));
  }
  .unlock-icon {
    display: grid;
    place-items: center;
    flex: 0 0 auto;
    width: 26px;
    height: 26px;
    border-radius: 50%;
    color: #0b1020;
    background: var(--extras);
    font-size: 14px;
    font-weight: 800;
  }
  .unlock-copy { display: grid; gap: 2px; min-width: 0; flex: 1 1 auto; font-size: 13px; }
  .unlock-copy span { color: var(--muted); }
  .unlock-link { color: var(--extras); font-size: 13px; font-weight: 650; white-space: nowrap; }
  .unlock-dismiss {
    flex: 0 0 auto;
    width: 28px;
    height: 28px;
    border: 0;
    border-radius: 8px;
    color: var(--muted);
    background: transparent;
    font-size: 20px;
    line-height: 1;
    cursor: pointer;
  }
  .unlock-dismiss:hover { color: var(--text); background: color-mix(in srgb, var(--text) 8%, transparent); }
  .filters { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; margin-bottom: 12px; }
  .filters .input { width: auto; }
  .search { flex: 1 1 240px; }
  .toggle { display: inline-flex; gap: 6px; align-items: center; min-height: 40px; color: var(--muted); padding: 0 6px; }
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
  .logo { position: relative; aspect-ratio: 16 / 10; display: grid; place-items: center; background: #1d2230; border-radius: 8px; overflow: hidden; }
  .logo img { max-width: 80%; max-height: 80%; object-fit: contain; }
  .name {
    font-size: 13px;
    font-weight: 600;
    line-height: 1.3;
    min-height: 2.6em; /* reserve two lines so cards align */
    overflow: hidden;
    overflow-wrap: anywhere;
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
  }
  .tags { display: flex; gap: 4px; min-height: 20px; margin-top: auto; }
  /* Extras/Premium pills overlay the logo corner. The logo area is dark in both
     themes, so these use fixed solid colours with high-contrast text. */
  .flags { position: absolute; top: 6px; right: 6px; display: flex; flex-direction: column; align-items: flex-end; gap: 3px; max-width: calc(100% - 12px); }
  .flag { padding: 1px 6px; border-radius: 999px; font-size: 10px; font-weight: 700; line-height: 1.4; letter-spacing: 0.02em; white-space: nowrap; }
  .flag-extras { background: #7ea3ff; color: #0b1020; }
  .flag-premium { background: #f4b740; color: #2a1c00; }
</style>
