'use client';
// Shared helpers for the ops console. Types are derived straight from the
// generated OpenAPI contract (paths) so they track the backend. A few ops
// routes are custom-action paths (e.g. `/v1/ops/kyc/{userId}:approve`) or use
// query-style filters that the generated client types as never/path-only; for
// those we call the client with a literal, already-resolved path so the request
// reaches the right URL. Everything else uses the fully typed client.
import type { paths } from '@kabados/api-client';
import { Button, Card, useApi, useAuth } from '@kabados/ui';
import type { KabadosClient } from '@kabados/api-client';
import { useEffect, useState, type ReactNode } from 'react';

// ---- Response types, pulled from the contract -----------------------------
type Json200<T> = T extends { responses: { 200: { content: { 'application/json': infer J } } } } ? J : never;

export type OpsConfig = Json200<paths['/v1/ops/config']['get']>;
export type KycItem = Json200<paths['/v1/ops/kyc']['get']>[number];
export type KycDocument = KycItem['documents'][number];
export type Dispute = Json200<paths['/v1/ops/disputes']['get']>[number];
export type DealEvidence = Json200<paths['/v1/ops/deals/{id}']['get']>;
export type DealLine = DealEvidence['lines'][number];
export type Weighing = DealEvidence['weighings'][number];
export type JobEvent = DealEvidence['job_events'][number];
export type AuditEntry = DealEvidence['audit'][number];
export type Rating = DealEvidence['ratings'][number];
export type AiReviewItem = Json200<paths['/v1/ops/ai-reviews']['get']>[number];
export type ListingView = Json200<paths['/v1/ops/listings/{id}']['get']>;
export type RateIndexEntry = Json200<paths['/v1/ops/rate-index:publish']['post']>[number];

// ---- Client side helpers ---------------------------------------------------
export function useMounted(): boolean {
  const [m, setM] = useState(false);
  useEffect(() => setM(true), []);
  return m;
}

/** Human message from a thrown API error (RFC 9457 problem details). */
export function msg(e: unknown): string {
  if (e && typeof e === 'object') {
    const p = e as { detail?: string; title?: string };
    if (typeof p.detail === 'string' && p.detail) return p.detail;
    if (typeof p.title === 'string' && p.title) return p.title;
  }
  return e instanceof Error ? e.message : 'Something went wrong';
}

/** GET a resolved ops path, optionally with query-string filters. */
export async function opsGet<T>(api: KabadosClient, path: string, query?: Record<string, string | null | undefined>): Promise<T> {
  const q: Record<string, string> = {};
  if (query) for (const [k, v] of Object.entries(query)) if (v != null && v !== '') q[k] = v;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const { data, error } = await (api.GET as any)(path, { params: { query: q } });
  if (error) throw error;
  return data as T;
}

/** POST a resolved ops action path (the action suffix is part of the path). */
export async function opsPost<T>(api: KabadosClient, path: string, body?: unknown): Promise<T> {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const init: any = {};
  if (body !== undefined) init.body = body;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const { data, error } = await (api.POST as any)(path, init);
  if (error) throw error;
  return data as T;
}

export function useOpsApi(): KabadosClient {
  return useApi();
}

/** Wraps a staff-only page: shows a sign-in prompt when there is no token. */
export function StaffGate({ children }: { children: ReactNode }) {
  const { token } = useAuth();
  const mounted = useMounted();
  if (!mounted) return null;
  if (!token)
    return (
      <Card title="Staff sign-in required">
        <p className="kb-muted">This is an internal console. Sign in with a staff account to continue.</p>
        <p><a href="/signin/"><Button>Staff sign in</Button></a></p>
      </Card>
    );
  return <>{children}</>;
}
