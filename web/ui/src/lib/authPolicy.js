// Which 401 responses mean "the admin session is gone" and should sign the
// viewer out. A wrong password or a wrong extras unlock code is also a 401, but
// it only says the attempt was rejected: the session is still valid.
export function signsOutOn401(path) {
  return !path.startsWith("/api/auth/") && !path.startsWith("/api/extras/unlock");
}
