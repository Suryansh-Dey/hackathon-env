// Helpers that bridge small gaps between the generated OpenAPI schema and the
// typed openapi-fetch client:
//   * the schema types path params as `never` and models vendor list filters as
//     path (not query) params, so `pth`/`qry` cast the params value. At runtime
//     openapi-fetch still substitutes `{id}` from `params.path` and serialises
//     `params.query` into the query string.
//   * several endpoints share one typed path but take an action suffix
//     (`:arrive`, `:simulate`, …). `as` on the path literal keeps the response
//     type while sending the real URL; the auth + idempotency middleware still
//     runs.

/** Path params for `{id}`-style routes (schema types them as `never`). */
export const pth = (p: Record<string, string>) => ({ path: p } as never);

/** Query-string filters the schema models as path params. */
export const qry = (q: Record<string, unknown>) => ({ query: q } as never);

/** The one typed deal-action path; every `:action` variant casts to it. */
export type DealActionPath = '/v1/deals/{id}:confirm-weights';

/** A typed no-body `{id}` POST, reused for the off-contract payment sim route. */
export type NoBodyIdPath = '/v1/deals/{id}/payment-request';

/** The typed rule-set version path; `:simulate` casts to the `:publish` key. */
export type VersionPath = '/v1/vendor/rule-sets/{id}/versions/{v}:publish';
