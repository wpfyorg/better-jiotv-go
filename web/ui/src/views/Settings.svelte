<script>
  import { onMount, onDestroy } from "svelte";
  import { api, loadChannels } from "../lib/api.js";
  import CopyField from "../lib/CopyField.svelte";
  import JioTVLogin from "../lib/JioTVLogin.svelte";
  import ExtrasLogin from "../lib/ExtrasLogin.svelte";
  import { plainHttpOrigin } from "../lib/origin.js";

  let status = $state(null);
  let error = $state("");
  let pw = $state({ current: "", next: "", message: "", ok: false });

  async function refresh() {
    try {
      status = await api("/api/status");
      error = "";
    } catch (err) {
      error = err.message;
    }
  }

  async function loginChanged() {
    await refresh();
    loadChannels(true);
  }

  async function logout(which) {
    if (!confirm(`Log out of ${which === "jiotv" ? "JioTV" : "the extra source"}?`)) return;
    try {
      await api(`/api/${which}/logout`, { method: "POST" });
      await loginChanged();
    } catch (err) {
      error = err.message;
    }
  }

  async function lockExtras() {
    try {
      await api("/api/extras/lock", { method: "POST" });
      await loginChanged();
    } catch (err) {
      error = err.message;
    }
  }

  // Typing the unlock code into the channel search box (Channels.svelte)
  // dispatches this once it succeeds, so this section can appear without a
  // reload.
  window.addEventListener("jiotv:extras-changed", refresh);
  onDestroy(() => window.removeEventListener("jiotv:extras-changed", refresh));

  async function rotateKey() {
    if (!confirm("Make a new access key? Every player using the current playlist URL will stop working until you give it the new one.")) return;
    try {
      status = await api("/api/key/rotate", { method: "POST" });
    } catch (err) {
      error = err.message;
    }
  }

  async function changePassword(event) {
    event.preventDefault();
    try {
      await api("/api/account/password", { method: "POST", body: { current: pw.current, new: pw.next } });
      pw = { current: "", next: "", message: "Password changed. Other browsers are signed out.", ok: true };
    } catch (err) {
      pw.message = err.message;
      pw.ok = false;
    }
  }

  // IPTV clients often reject a self-signed certificate, so when this page was
  // opened on the server's own HTTPS port, offer the plain-HTTP origin instead.
  let iptvOrigin = $derived(plainHttpOrigin(location, status));

  onMount(refresh);
</script>

{#if error}<p class="error" role="alert">{error}</p>{/if}

{#if status}
  <div class="cols">
    <section class="card">
      <h2>IPTV playlist</h2>
      <p class="muted">Add these to your TV's IPTV app. Anyone with the playlist URL can watch, so keep it private.</p>
      <CopyField label="Playlist" value={iptvOrigin + status.playlistPath} />
      <CopyField label="EPG" value={iptvOrigin + status.epgPath} />
      {#if !status.epg}<p class="muted small">The EPG file is off on this server (option <code>epg</code>).</p>{/if}
      <button class="btn danger" onclick={rotateKey}>Make a new key</button>
    </section>

    <section class="card">
      <h2>JioTV</h2>
      {#if status.jiotv.loggedIn}
        <p class="ok">Logged in</p>
        {#if !status.logoutDisabled}<button class="btn" onclick={() => logout("jiotv")}>Log out</button>{/if}
      {:else}
        <p class="muted">Not logged in. {status.extras.connected ? "Channels the extra source carries play through it." : ""}</p>
        <JioTVLogin ondone={loginChanged} />
      {/if}
    </section>

    {#if status.extras.enabled}
      <section class="card">
        <h2>Extra channels</h2>
        {#if status.extras.connected}
          <p class="ok">Connected</p>
          {#if !status.logoutDisabled}<button class="btn" onclick={() => logout("extras")}>Disconnect</button>{/if}
        {:else}
          <p class="muted">Use the registered mobile number for the extra source.</p>
          <ExtrasLogin ondone={loginChanged} />
        {/if}
        <button class="btn" onclick={lockExtras}>Lock</button>
      </section>
    {/if}

    <section class="card">
      <h2>Admin password</h2>
      <form onsubmit={changePassword}>
        <input class="input" type="password" placeholder="Current password" autocomplete="current-password" bind:value={pw.current} required />
        <input class="input" type="password" placeholder="New password (8+ characters)" autocomplete="new-password" minlength="8" bind:value={pw.next} required />
        <button class="btn">Change password</button>
        <p class:ok={pw.ok} class:error={!pw.ok} role="status">{pw.message}</p>
      </form>
    </section>
  </div>
  <p class="muted small">JioTV Go {status.version}</p>
{:else if !error}
  <p class="muted">Loading…</p>
{/if}

<style>
  .cols { display: grid; gap: 16px; grid-template-columns: repeat(auto-fit, minmax(320px, 1fr)); align-items: start; }
  section { display: flex; flex-direction: column; gap: 12px; align-items: flex-start; }
  section > :global(*) { max-width: 100%; }
  h2 { margin: 0; font-size: 17px; }
  p { margin: 0; }
  form { display: flex; flex-direction: column; gap: 10px; width: 100%; align-items: flex-start; }
  .small { font-size: 13px; margin-top: 16px; }
  code { background: var(--surface-2); padding: 1px 5px; border-radius: 5px; }
</style>
