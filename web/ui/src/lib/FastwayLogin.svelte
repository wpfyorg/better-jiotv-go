<script>
  import { api } from "./api.js";

  let { ondone } = $props();
  let username = $state("");
  let password = $state("");
  let message = $state("");
  let busy = $state(false);

  async function connect(event) {
    event.preventDefault();
    busy = true;
    message = "";
    try {
      const d = await api("/api/fastway/login", {
        method: "POST",
        body: { username: username.trim(), password },
      });
      if (d?.status) ondone();
      else message = "Couldn't connect Fastway. Check the username and password.";
    } catch (err) {
      message = err.message;
    } finally {
      busy = false;
    }
  }
</script>

<form onsubmit={connect}>
  <input
    class="input"
    type="text"
    autocomplete="username"
    placeholder="Fastway username"
    bind:value={username}
    required
  />
  <input
    class="input"
    type="password"
    autocomplete="current-password"
    placeholder="Password"
    bind:value={password}
    required
  />
  <button class="btn primary" disabled={busy}>Connect</button>
</form>
<p class="error" role="alert">{message}</p>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: 100%;
    align-items: flex-start;
  }
  p {
    margin: 0;
  }
</style>
