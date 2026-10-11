// Shared helpers for the unauthenticated public site (PLAN §10.2).
// This app has no Providers/useApi, so we talk to the API with plain fetch.
import { apiBaseUrl } from '@kabados/ui';
import type { paths } from '@kabados/api-client';

// NOTE: the generated schema has two operations keyed "config", so the
// paths-derived type for /v1/public/config is ambiguous. We only read
// `cities` here (ClientConfig.cities in the contract), so type it minimally.
export type ClientConfig = { cities?: string[] | null };
export type RateIndexEntry = paths['/v1/public/rate-index']['get']['responses'][200]['content']['application/json'][number];
export type CityImpact = NonNullable<paths['/v1/public/impact']['get']['responses'][200]['content']['application/json']>;
export type GreenReceipt = paths['/v1/public/receipts/{id}']['get']['responses'][200]['content']['application/json'];

/** Default city list used until GET /v1/public/config loads (or if it fails). */
export const FALLBACK_CITIES = ['delhi', 'mumbai', 'bengaluru', 'chennai', 'hyderabad', 'kolkata', 'pune'];

/** "bengaluru" -> "Bengaluru". */
export function cityLabel(city: string): string {
  return city.replace(/\b\w/g, (c) => c.toUpperCase());
}

export class ApiError extends Error {
  status: number;
  constructor(message: string, status: number) {
    super(message);
    this.status = status;
  }
}

/** GET a JSON path off the public API base. Throws ApiError on a non-2xx. */
export async function fetchJson<T>(path: string, signal?: AbortSignal): Promise<T> {
  const res = await fetch(`${apiBaseUrl()}${path}`, {
    signal,
    headers: { accept: 'application/json' },
  });
  if (!res.ok) {
    let detail = '';
    try {
      const body = (await res.json()) as { detail?: string; title?: string };
      detail = body.detail || body.title || '';
    } catch {
      /* non-JSON error body */
    }
    throw new ApiError(detail || `Request failed (${res.status})`, res.status);
  }
  return (await res.json()) as T;
}

/** Read a single query param on the client (empty on the server during export). */
export function queryParam(name: string): string {
  if (typeof window === 'undefined') return '';
  return new URLSearchParams(window.location.search).get(name) ?? '';
}
