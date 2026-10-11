'use client';
// TanStack Query provider + a shared KabadOS API client bound to the current
// token. One place configures retries/stale time for every site.
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { createApi, type KabadosClient } from '@kabados/api-client';
import { createContext, useContext, useMemo, useRef, type ReactNode } from 'react';
import { useAuth } from './auth';

const ApiCtx = createContext<KabadosClient | null>(null);

export function apiBaseUrl(): string {
  // Set at build time per app; falls back to the local dev server.
  return process.env.NEXT_PUBLIC_API_URL ?? 'http://localhost:8080';
}

export function Providers({ children }: { children: ReactNode }) {
  const { token, signOut } = useAuth();
  const qc = useRef(new QueryClient({ defaultOptions: { queries: { retry: 1, staleTime: 15_000, refetchOnWindowFocus: false } } }));
  const api = useMemo(
    () => createApi({ baseUrl: apiBaseUrl(), getToken: () => token, onUnauthorized: signOut }),
    [token, signOut],
  );
  return (
    <QueryClientProvider client={qc.current}>
      <ApiCtx.Provider value={api}>{children}</ApiCtx.Provider>
    </QueryClientProvider>
  );
}

export function useApi(): KabadosClient {
  const v = useContext(ApiCtx);
  if (!v) throw new Error('useApi must be used within Providers');
  return v;
}
