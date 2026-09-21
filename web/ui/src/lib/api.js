// Small fetch wrapper for the JioTV Go API.

export class ApiError extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}

// The /k/<key>/ prefix when the page was opened through a key link.
export const keyBase = location.pathname.match(/^\/k\/[0-9a-f]{32}\//)?.[0] ?? null;

export async function api(path, { method = "GET", body } = {}) {
  const res = await fetch(path, {
    method,
    credentials: "same-origin",
    headers: body === undefined ? {} : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  let data = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    data = { message: text };
  }
  if (!res.ok) {
    if (res.status === 401 && !path.startsWith("/api/auth/")) {
      window.dispatchEvent(new CustomEvent("jiotv:signed-out"));
    }
    throw new ApiError(res.status, data?.message || res.statusText);
  }
  return data;
}

let channelsPromise = null;

// Channels are fetched once per page load.
export function loadChannels(force = false) {
  if (force || !channelsPromise) {
    channelsPromise = api("/api/channels").then((d) => d.channels);
    channelsPromise.catch(() => (channelsPromise = null));
  }
  return channelsPromise;
}

// Shape of the extras unlock code (see docs/config.md and src/unlock.rs).
// Matched before ever sending a search-box query to the server, so an
// ordinary channel search never leaves the browser.
export function looksLikeUnlockCode(s) {
  return /^\d{1,3}[a-z]{3,9}\d{1,2}a\d{1,3}k\d{1,3}n\d{1,3}$/i.test(s.trim());
}

export function formatTime(ms) {
  return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}
