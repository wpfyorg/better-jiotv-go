<script>
  let { label, value } = $props();
  let copied = $state(false);

  async function copy() {
    try {
      await navigator.clipboard.writeText(value);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {
      copied = false;
    }
  }
</script>

<div class="field">
  <span class="label">{label}</span>
  <div class="row">
    <input class="input" readonly {value} aria-label={label} onfocus={(e) => e.currentTarget.select()} />
    <button class="btn" type="button" onclick={copy}>{copied ? "Copied" : "Copy"}</button>
  </div>
</div>

<style>
  .field { width: 100%; display: flex; flex-direction: column; gap: 4px; }
  .label { font-size: 12px; font-weight: 600; color: var(--muted); }
  .row { display: flex; gap: 8px; }
  .input { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 12px; }
</style>
