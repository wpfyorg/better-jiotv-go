<script>
  import { onMount } from "svelte";
  import { api } from "./lib/api.js";
  import { route } from "./lib/router.svelte.js";
  import Login from "./views/Login.svelte";
  import Channels from "./views/Channels.svelte";
  import Watch from "./views/Watch.svelte";
  import Settings from "./views/Settings.svelte";

  let auth = $state({ loading: true, passwordSet: false, authenticated: false });

  async function refreshAuth() {
    try {
      const s = await api("/api/auth/state");
      auth = { loading: false, ...s };
    } catch {
      auth = { loading: false, passwordSet: true, authenticated: false };
    }
  }

  async function signOut() {
    await api("/api/auth/logout", { method: "POST" }).catch(() => {});
    auth = { ...auth, authenticated: false };
  }

  onMount(() => {
    refreshAuth();
    const onSignedOut = () => (auth = { ...auth, authenticated: false });
    window.addEventListener("jiotv:signed-out", onSignedOut);
    return () => window.removeEventListener("jiotv:signed-out", onSignedOut);
  });
</script>

{#if auth.loading}
  <p class="muted center">Loading…</p>
{:else if !auth.authenticated}
  <Login passwordSet={auth.passwordSet} onsignedin={refreshAuth} />
{:else}
  <header class="bar">
    <a class="brand" href="#/">JioTV Go</a>
    <nav>
      <a href="#/" aria-current={route.name === "channels" ? "page" : undefined}>Channels</a>
      <a href="#/settings" aria-current={route.name === "settings" ? "page" : undefined}>Settings</a>
      <button class="link" onclick={signOut}>Sign out</button>
    </nav>
  </header>
  <main>
    {#if route.name === "watch"}
      <Watch id={route.param} />
    {:else if route.name === "settings"}
      <Settings />
    {:else}
      <Channels />
    {/if}
  </main>
{/if}

<style>
  .center { text-align: center; margin-top: 30vh; }
  .bar {
    position: sticky;
    top: 0;
    z-index: 10;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 12px 20px;
    background: color-mix(in srgb, var(--bg) 88%, transparent);
    backdrop-filter: blur(8px);
    border-bottom: 1px solid var(--border);
  }
  .brand { font-weight: 800; font-size: 18px; text-decoration: none; }
  nav { display: flex; align-items: center; gap: 4px; }
  nav a, .link {
    padding: 6px 12px;
    border-radius: 8px;
    text-decoration: none;
    color: var(--muted);
    background: none;
    border: 0;
    cursor: pointer;
  }
  nav a[aria-current="page"] { color: var(--text); background: var(--surface-2); }
  nav a:hover, .link:hover { color: var(--text); }
  main { max-width: 1400px; margin: 0 auto; padding: 20px 16px 48px; }
</style>
