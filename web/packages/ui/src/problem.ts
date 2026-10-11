// Turns any thrown value into a message for the user. openapi-fetch hands back
// RFC 9457 problem objects ({ title, detail, status }) rather than Error
// instances, so prefer the server's `detail`, then `title`, then the fallback.

export interface ProblemLike {
  type?: string;
  title?: string;
  status?: number;
  detail?: string;
}

/** True when `e` looks like an RFC 9457 problem object from the API. */
export function isProblem(e: unknown): e is ProblemLike {
  return !!e && typeof e === 'object' && !(e instanceof Error) && ('detail' in e || 'title' in e || 'status' in e);
}

/** HTTP status of a problem object, if any. */
export function problemStatus(e: unknown): number | undefined {
  return isProblem(e) && typeof e.status === 'number' ? e.status : undefined;
}

/** The best user-facing message for an error: problem.detail, Error.message, or `fallback`. */
export function problemMessage(e: unknown, fallback = 'Something went wrong'): string {
  if (isProblem(e)) {
    const msg = (typeof e.detail === 'string' && e.detail.trim()) || (typeof e.title === 'string' && e.title.trim());
    return msg || fallback;
  }
  if (e instanceof Error && e.message) return e.message;
  if (typeof e === 'string' && e.trim()) return e;
  return fallback;
}
