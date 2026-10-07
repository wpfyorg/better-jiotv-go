<script>
  import { api } from "./api.js";

  let { ondone } = $props();
  let step = $state("number");
  let number = $state("");
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
      const d = await api("/login/sendOTP", { method: "POST", body: { number: "+91" + number } });
      if (d?.status) step = "otp";
      else message = "Couldn't send the OTP. Check the number.";
    });
  };

  const verify = (e) => {
    e.preventDefault();
    run(async () => {
      const d = await api("/login/verifyOTP", { method: "POST", body: { number: "+91" + number, otp } });
      if (d?.status && d.status !== "failed") ondone();
      else message = "The OTP is wrong or has expired.";
    });
  };
</script>

{#if step === "number"}
  <form onsubmit={sendOTP}>
    <input class="input" type="tel" inputmode="numeric" placeholder="10-digit Jio mobile number" pattern={'[0-9]{10}'} maxlength="10" bind:value={number} required />
    <button class="btn primary" disabled={busy}>Send OTP</button>
  </form>
{:else}
  <form onsubmit={verify}>
    <input class="input" inputmode="numeric" autocomplete="one-time-code" placeholder="OTP" bind:value={otp} required />
    <div class="row">
      <button class="btn primary" disabled={busy}>Log in</button>
      <button class="btn" type="button" onclick={() => (step = "number")}>Back</button>
    </div>
  </form>
{/if}
<p class="error" role="alert">{message}</p>

<style>
  form { display: flex; flex-direction: column; gap: 10px; width: 100%; align-items: flex-start; }
  .row { display: flex; gap: 8px; }
  p { margin: 0; }
</style>
