'use client';
import { AuthProvider, Providers as QueryProviders } from '@kabados/ui';
import type { ReactNode } from 'react';
export function AppProviders({ children }: { children: ReactNode }) {
  return (
    <AuthProvider>
      <QueryProviders>{children}</QueryProviders>
    </AuthProvider>
  );
}
