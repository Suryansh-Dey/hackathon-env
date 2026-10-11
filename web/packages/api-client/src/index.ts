// Typed KabadOS API client, generated from the backend's OpenAPI contract.
// Injects the bearer token and a per-request Idempotency-Key on writes.
import createClient, { type Client, type Middleware } from 'openapi-fetch';
import type { paths } from './schema';

export type { paths } from './schema';
export type KabadosClient = Client<paths>;

export interface ApiOptions {
  baseUrl: string;
  /** Returns the current bearer token (Cognito or dev token), or null. */
  getToken?: () => string | null | undefined;
  /** Called on 401 so the app can route to sign-in. */
  onUnauthorized?: () => void;
}

/** A short, unique Idempotency-Key for a write request. */
export function newIdempotencyKey(): string {
  if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) return `web-${crypto.randomUUID()}`;
  return `web-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

export function createApi(opts: ApiOptions): KabadosClient {
  const client = createClient<paths>({ baseUrl: opts.baseUrl });

  const auth: Middleware = {
    onRequest({ request }) {
      const token = opts.getToken?.();
      if (token) request.headers.set('authorization', `Bearer ${token}`);
      if (request.method === 'POST' && !request.headers.has('idempotency-key')) {
        request.headers.set('idempotency-key', newIdempotencyKey());
      }
      return request;
    },
    onResponse({ response }) {
      if (response.status === 401) opts.onUnauthorized?.();
      return response;
    },
  };
  client.use(auth);
  return client;
}

/** RFC 9457 problem details returned by the API on error. */
export interface Problem {
  type: string;
  title: string;
  status: number;
  detail?: string;
  errors?: { path: string; message: string }[];
}

export function problemMessage(error: unknown): string {
  const p = error as Problem | undefined;
  if (p && typeof p === 'object' && 'title' in p) return p.detail ?? p.title ?? 'Something went wrong';
  return 'Something went wrong';
}
