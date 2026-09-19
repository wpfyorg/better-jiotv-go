<script>
  import { untrack } from "svelte";
  import { api } from "../lib/api.js";
  import Rail from "../lib/Rail.svelte";

  const screens = [
    { id: "1", name: "Home" },
    { id: "100021", name: "Movies" },
    { id: "100023", name: "Shows" },
    { id: "100025", name: "Kids" },
    { id: "100097", name: "TV shows" },
  ];
  const providers = [
    { id: "", name: "All" },
    { id: "JioCinema", name: "JioCinema" },
    { id: "MXPlayer", name: "MX Player" },
    { id: "Zee5", name: "ZEE5" },
  ];

  let screen = $state(sessionStorage.getItem("ottScreen") || "1");
  let provider = $state(sessionStorage.getItem("ottProvider") || "");
  let query = $state("");
  let searched = $state("");
  let rails = $state([]);
  let page = $state(0);
  let more = $state(false);
  let loading = $state(false);
  let error = $state("");

  $effect(() => {
    sessionStorage.setItem("ottScreen", screen);
    sessionStorage.setItem("ottProvider", provider);
  });

  async function loadPage(reset) {
    loading = true;
    error = "";
    try {
      const next = reset ? 0 : page + 1;
      // Screens return five rows per page and many rows have none of our
      // providers, so keep going until something shows or the screen ends.
      let got = [];
      let p = next;
      let hasMore = true;
      while (hasMore && got.length === 0 && p < next + 4) {
        const d = await api(`/api/ott/screen/${screen}?page=${p}`);
        got = d.rails;
        hasMore = d.more;
        p++;
      }
      rails = reset ? got : [...rails, ...got];
      page = p - 1;
      more = hasMore;
    } catch (err) {
      error = err.message;
    } finally {
      loading = false;
    }
  }

  async function search(event) {
    event.preventDefault();
    const q = query.trim();
    if (!q) {
      searched = "";
      loadPage(true);
      return;
    }
    loading = true;
    error = "";
    try {
      rails = (await api("/api/ott/search?q=" + encodeURIComponent(q))).rails;
      searched = q;
      more = false;
    } catch (err) {
      error = err.message;
    } finally {
      loading = false;
    }
  }

  // Reload when the section changes; nothing else should re-run this.
  $effect(() => {
    screen;
    untrack(() => {
      searched = "";
      query = "";
      loadPage(true);
    });
  });

  const visibleCount = $derived(
    rails.reduce((n, r) => n + (provider ? r.items.filter((i) => i.provider === provider).length : r.items.length), 0),
  );
</script>

<div class="top">
  <nav class="tabs" aria-label="Sections">
    {#each screens as s}
      <button class:active={screen === s.id && !searched} onclick={() => (screen = s.id)}>{s.name}</button>
    {/each}
  </nav>
  <form onsubmit={search} role="search">
    <input class="input" type="search" placeholder="Search movies and shows" bind:value={query} aria-label="Search movies and shows" />
  </form>
</div>

<div class="chips" role="group" aria-label="Provider">
  {#each providers as p}
    <button class="chip" aria-pressed={provider === p.id} onclick={() => (provider = p.id)}>{p.name}</button>
  {/each}
</div>

{#if searched}<p class="muted">Results for “{searched}”</p>{/if}
{#if error}<p class="error" role="alert">{error}</p>{/if}

{#each rails as r, i (i + r.title)}
  <Rail title={r.title} items={r.items} {provider} />
{/each}

{#if !loading && !error && visibleCount === 0}
  <p class="muted">Nothing from {provider ? providers.find((p) => p.id === provider).name : "JioCinema, MX Player or ZEE5"} here{more ? " yet" : ""}.</p>
{/if}
{#if loading}
  <p class="muted">Loading…</p>
{:else if more && !searched}
  <button class="btn" onclick={() => loadPage(false)}>Load more</button>
{/if}

<style>
  .top { display: flex; flex-wrap: wrap; gap: 12px; justify-content: space-between; align-items: center; margin-bottom: 12px; }
  .tabs { display: flex; gap: 4px; flex-wrap: wrap; }
  .tabs button {
    padding: 7px 14px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: none;
    color: var(--muted);
    cursor: pointer;
  }
  .tabs button.active { background: var(--text); color: var(--bg); border-color: var(--text); }
  form { flex: 1 1 260px; max-width: 420px; }
  .chips { display: flex; gap: 6px; flex-wrap: wrap; margin-bottom: 18px; }
  .chip {
    padding: 5px 12px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--muted);
    cursor: pointer;
    font-size: 13px;
  }
  .chip[aria-pressed="true"] { color: var(--accent); border-color: var(--accent); }
</style>
