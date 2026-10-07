// IPTV clients often reject the generated self-signed certificate, so when the
// page was opened on the server's own HTTPS port the playlist and EPG should be
// offered on the plain-HTTP origin. Anywhere else (plain HTTP, or a tunnel/proxy
// origin that is not the local TLS port) the page's own origin is the right one.
export function plainHttpOrigin(loc, status) {
  const http = status?.httpPort;
  const tls = status?.tlsPort;
  // Browsers report an empty port for the scheme's default (443 for HTTPS).
  const port = loc.port || (loc.protocol === "https:" ? "443" : "");
  if (loc.protocol === "https:" && http && tls && port === String(tls)) {
    return `http://${loc.hostname}:${http}`;
  }
  return loc.origin;
}
