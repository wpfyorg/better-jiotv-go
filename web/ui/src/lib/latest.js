// Hands out request tokens so that only the most recently started request may
// publish its result: `start()` returns a function that is true until another
// request has started. A slow earlier request then cannot overwrite what a
// newer one already loaded, nor restore an error the newer one cleared.
export function latestOnly() {
  let latest = 0;
  return {
    start() {
      const mine = ++latest;
      return () => mine === latest;
    },
  };
}
