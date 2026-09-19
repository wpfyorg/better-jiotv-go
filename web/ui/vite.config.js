import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// The Go server serves index.html at / and the assets under /ui/.
// `npm run dev` proxies API and stream routes to a local server on :5001.
const backend = process.env.JIOTV_BACKEND || "http://127.0.0.1:5001";
const proxied = ["/api", "/k", "/login", "/tvplus", "/epg", "/mpd", "/player", "/live", "/render", "/drm", "/dashtime", "/jtvimage", "/static"];

export default defineConfig({
  base: "/ui/",
  plugins: [svelte()],
  build: { outDir: "dist", emptyOutDir: true, assetsInlineLimit: 0 },
  server: { proxy: Object.fromEntries(proxied.map((p) => [p, backend])) },
});
