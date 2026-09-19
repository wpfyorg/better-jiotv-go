// Hash router: #/, #/watch/<id>, #/settings.

function parse() {
  const hash = location.hash.replace(/^#/, "") || "/";
  const [, first = "", ...rest] = hash.split("/");
  return { name: first || "channels", param: decodeURIComponent(rest.join("/")) };
}

export const route = $state(parse());

window.addEventListener("hashchange", () => Object.assign(route, parse()));

export function go(path) {
  location.hash = path;
}
