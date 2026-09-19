// Loads a classic script once and resolves when it has run.
const loaded = new Map();

export function loadScript(src) {
  if (!loaded.has(src)) {
    loaded.set(
      src,
      new Promise((resolve, reject) => {
        const s = document.createElement("script");
        s.src = src;
        s.onload = resolve;
        s.onerror = () => {
          loaded.delete(src);
          reject(new Error("could not load " + src));
        };
        document.head.appendChild(s);
      }),
    );
  }
  return loaded.get(src);
}
