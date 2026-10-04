<script>
  import { onMount } from "svelte";
  import { api } from "./lib/api.js";
  import { route } from "./lib/router.svelte.js";
  import Login from "./views/Login.svelte";
  import Channels from "./views/Channels.svelte";
  import Watch from "./views/Watch.svelte";
  import Settings from "./views/Settings.svelte";
  import OnDemand from "./views/OnDemand.svelte";
  import Show from "./views/Show.svelte";
  import VodPlayer from "./views/VodPlayer.svelte";

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
      <a href="#/ott" aria-current={["ott", "show", "play"].includes(route.name) ? "page" : undefined}>On demand</a>
      <a href="#/settings" aria-current={route.name === "settings" ? "page" : undefined}>Settings</a>
      <button class="link" onclick={signOut}>Sign out</button>
    </nav>
  </header>
  <main>
    {#if route.name === "watch"}
      <Watch id={route.param} />
    {:else if route.name === "ott"}
      <OnDemand />
    {:else if route.name === "show"}
      <Show id={route.param} />
    {:else if route.name === "play"}
      <VodPlayer id={route.param} />
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
    min-width: 0;
    padding: 11px max(16px, calc((100vw - 1500px) / 2 + 16px));
    background: color-mix(in srgb, var(--bg) 88%, transparent);
    backdrop-filter: blur(14px);
    border-bottom: 1px solid var(--border);
  }
  .brand { flex: 0 0 auto; font-weight: 800; font-size: 17px; letter-spacing: -.025em; text-decoration: none; }
  nav {
    display: flex;
    min-width: 0;
    align-items: center;
    /* flex-end would clip the first links when the row overflows on narrow phones. */
    justify-content: flex-start;
    gap: 3px;
    overflow-x: auto;
    scrollbar-width: none;
  }
  nav::-webkit-scrollbar { display: none; }
  nav a, .link {
    flex: 0 0 auto;
    padding: 6px 10px;
    border-radius: 8px;
    text-decoration: none;
    color: var(--muted);
    background: none;
    border: 0;
    cursor: pointer;
    font-size: 13px;
    font-weight: 550;
  }
  nav a[aria-current="page"] { color: var(--text); background: var(--surface-2); }
  nav a:hover, .link:hover { color: var(--text); }
  main { width: 100%; max-width: 1500px; margin: 0 auto; padding: 24px 18px 56px; }

  @media (max-width: 640px) {
    .bar { gap: 10px; padding: 9px 12px; }
    .brand { font-size: 15px; }
    nav a, .link { display: inline-flex; align-items: center; min-height: 40px; padding: 0 8px; font-size: 12px; }
    main { padding: 14px 10px 40px; }
  }
  @media (max-width: 380px) {
    .bar { gap: 6px; }
    nav { gap: 0; }
    nav a, .link { padding: 0 6px; }
  }
</style>
