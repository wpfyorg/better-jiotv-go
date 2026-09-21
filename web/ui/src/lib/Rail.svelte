<script>
  let { title, items, provider = "" } = $props();

  const shown = $derived(provider ? items.filter((i) => i.provider === provider) : items);

  function href(item) {
    return item.contentType === "Show" ? "#/show/" + item.contentId : "#/play/" + item.contentId;
  }
</script>

{#if shown.length}
  <section>
    <h2>{title}</h2>
    <ul>
      {#each shown as item (item.contentId)}
        <li>
          <a href={href(item)} title={item.description || item.name}>
            <span class="art"><img src={item.thumbnail} alt="" loading="lazy" decoding="async" /></span>
            <span class="name">{item.name}</span>
            <span class="meta muted">
              {item.provider}{item.contentType === "Show" ? " · Show" : ""}{item.language ? " · " + item.language : ""}
            </span>
          </a>
        </li>
      {/each}
    </ul>
  </section>
{/if}

<style>
  section { margin-bottom: 26px; }
  h2 { font-size: 16px; margin: 0 0 10px; }
  ul {
    list-style: none;
    margin: 0;
    padding: 0 0 6px;
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: minmax(220px, 260px);
    gap: 12px;
    overflow-x: auto;
    scroll-snap-type: x proximity;
  }
  li { scroll-snap-align: start; }
  a { display: flex; flex-direction: column; gap: 6px; text-decoration: none; border-radius: var(--radius); padding: 4px; }
  a:hover .art, a:focus-visible .art { outline: 2px solid var(--accent); }
  .art { aspect-ratio: 16 / 9; border-radius: 10px; overflow: hidden; background: #1d2230; display: block; }
  .art img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .name { font-size: 14px; font-weight: 600; line-height: 1.3; }
  .meta { font-size: 12px; }
</style>
