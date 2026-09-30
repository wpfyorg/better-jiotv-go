<script>
  import { api, keyBase } from "../lib/api.js";

  let { passwordSet, onsignedin } = $props();

  let password = $state("");
  let confirm = $state("");
  let message = $state("");
  let busy = $state(false);

  const setup = !passwordSet;

  async function submit(event) {
    event.preventDefault();
    message = "";
    if (setup && password !== confirm) {
      message = "The passwords don't match.";
      return;
    }
    busy = true;
    try {
      if (setup) {
        await api(keyBase + "api/auth/setup", { method: "POST", body: { password } });
        // Leave the key link so it doesn't stay in the address bar.
        history.replaceState(null, "", "/");
      } else {
        await api("/api/auth/login", { method: "POST", body: { password } });
      }
      password = confirm = "";
      onsignedin();
    } catch (err) {
      message = err.message;
    } finally {
      busy = false;
    }
  }
</script>

<div class="wrap">
  <form class="card" onsubmit={submit}>
    <h1>JioTV Go</h1>
    {#if setup && !keyBase}
      <p>No admin password is set yet.</p>
      <p class="muted">
        Open the setup link the server printed when it started (it contains the access key), or run
        <code>jiotv admin password</code> on the server.
      </p>
    {:else}
      <p class="muted">{setup ? "Choose an admin password for this server." : "Enter the admin password."}</p>
      <label>
        <span class="sr-only">Password</span>
        <input
          class="input"
          type="password"
          placeholder="Password"
          autocomplete={setup ? "new-password" : "current-password"}
          bind:value={password}
          required
          minlength={setup ? 8 : undefined}
        />
      </label>
      {#if setup}
        <label>
          <span class="sr-only">Repeat password</span>
          <input class="input" type="password" placeholder="Repeat password" autocomplete="new-password" bind:value={confirm} required />
        </label>
      {/if}
      <button class="btn primary" disabled={busy}>{setup ? "Set password" : "Sign in"}</button>
      <p class="error" role="alert">{message}</p>
    {/if}
  </form>
</div>

<style>
  .wrap { min-height: 100vh; display: grid; place-items: center; padding: 16px; }
  form { width: min(380px, 100%); display: flex; flex-direction: column; gap: 12px; }
  h1 { margin: 0; font-size: 24px; }
  p { margin: 0; }
  code { background: var(--surface-2); padding: 1px 5px; border-radius: 5px; }
</style>
