// Which 401 responses mean "the admin session is gone" and should sign the
// viewer out. A wrong password is a 401 that only says the attempt was
// rejected, and so is a wrong extras unlock code. But the unlock endpoint sits
// behind the same access gate as everything else, whose own 401 ("unauthorized")
// does mean the session expired, so only the endpoint's "wrong code" answer is
// exempt, not the whole path.
export function signsOutOn401(path, message = "") {
  if (path.startsWith("/api/auth/")) return false;
  if (path.startsWith("/api/extras/unlock")) return message !== "wrong code";
  return true;
}
