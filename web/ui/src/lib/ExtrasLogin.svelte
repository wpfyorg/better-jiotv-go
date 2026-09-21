<script>
  import { api } from "./api.js";

  let { ondone } = $props();
  let step = $state("number");
  let number = $state("");
  let connections = $state([]);
  let picked = $state(0);
  let otp = $state("");
  let message = $state("");
  let busy = $state(false);

  async function run(fn) {
    busy = true;
    message = "";
    try {
      await fn();
    } catch (err) {
      message = err.message;
    } finally {
      busy = false;
    }
  }

  const sendOTP = (e) => {
    e.preventDefault();
    run(async () => {
      const d = await api("/tvplus/login/sendOTP", { method: "POST", body: { number } });
      if (d?.connections?.length) {
        connections = d.connections;
        picked = d.connections[0].index;
        step = "connection";
      } else if (d?.status) {
        step = "otp";
      } else {
        message = "Couldn't send the OTP. Check the number.";
      }
    });
  };

  const choose = (e) => {
    e.preventDefault();
    run(async () => {
      const d = await api("/tvplus/login/sendOTP", { method: "POST", body: { number, connection: picked } });
      if (d?.status) step = "otp";
      else message = "Couldn't send the OTP.";
    });
  };

  const verify = (e) => {
    e.preventDefault();
    run(async () => {
      const d = await api("/tvplus/login/verifyOTP", { method: "POST", body: { number, otp } });
      if (d?.status) ondone();
      else message = "The OTP is wrong, or this connection has no JioTV+ plan.";
    });
  };
</script>

{#if step === "number"}
  <form onsubmit={sendOTP}>
    <input class="input" type="tel" inputmode="numeric" placeholder="10-digit mobile number" pattern="[0-9]{10}" maxlength="10" bind:value={number} required />
    <button class="btn primary" disabled={busy}>Continue</button>
  </form>
{:else if step === "connection"}
  <form onsubmit={choose}>
    <fieldset>
      <legend class="muted">Choose the connection</legend>
      {#each connections as conn}
        <label class="conn">
          <input type="radio" name="connection" value={conn.index} bind:group={picked} />
          <span>{[conn.name, conn.product, conn.lineEndsWith && "line ending " + conn.lineEndsWith].filter(Boolean).join(" · ")}</span>
        </label>
      {/each}
    </fieldset>
    <button class="btn primary" disabled={busy}>Send OTP</button>
  </form>
{:else}
  <form onsubmit={verify}>
    <input class="input" inputmode="numeric" autocomplete="one-time-code" placeholder="OTP" bind:value={otp} required />
    <div class="row">
      <button class="btn primary" disabled={busy}>Connect</button>
      <button class="btn" type="button" onclick={() => (step = "number")}>Start again</button>
    </div>
  </form>
{/if}
<p class="error" role="alert">{message}</p>

<style>
  form { display: flex; flex-direction: column; gap: 10px; width: 100%; align-items: flex-start; }
  fieldset { border: 0; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 6px; width: 100%; }
  legend { margin-bottom: 4px; }
  .conn { display: flex; gap: 8px; align-items: center; padding: 8px 10px; border: 1px solid var(--border); border-radius: 10px; cursor: pointer; }
  .row { display: flex; gap: 8px; }
  p { margin: 0; }
</style>
