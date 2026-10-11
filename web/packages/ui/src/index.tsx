// Shared presentational components. Minimal, dependency-free, theme-aware.
export * from './format';
export { AuthProvider, useAuth } from './auth';
export { Providers, useApi, apiBaseUrl } from './query';
export { PhotoUpload } from './photo';
export type { PhotoUploadProps } from './photo';

import type { ReactNode, ButtonHTMLAttributes } from 'react';

export function Button({ variant = 'primary', children, ...props }: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: 'primary' | 'ghost' | 'danger' }) {
  const cls = { primary: 'kb-btn kb-btn-primary', ghost: 'kb-btn kb-btn-ghost', danger: 'kb-btn kb-btn-danger' }[variant];
  return (
    <button className={cls} {...props}>
      {children}
    </button>
  );
}

export function Card({ children, title }: { children: ReactNode; title?: string }) {
  return (
    <section className="kb-card">
      {title ? <h2 className="kb-card-title">{title}</h2> : null}
      {children}
    </section>
  );
}

export function Badge({ children, tone = 'neutral' }: { children: ReactNode; tone?: 'neutral' | 'good' | 'warn' | 'hazard' }) {
  return <span className={`kb-badge kb-badge-${tone}`}>{children}</span>;
}

export function Spinner({ label = 'Loading…' }: { label?: string }) {
  return <p className="kb-muted" role="status">{label}</p>;
}

export function ErrorNote({ children }: { children: ReactNode }) {
  return <p className="kb-error" role="alert">{children}</p>;
}

export function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="kb-field">
      <span className="kb-field-label">{label}</span>
      {children}
    </label>
  );
}

/** Verification ladder label (L0–L4). */
export function LadderBadge({ level }: { level: string }) {
  const map: Record<string, { text: string; tone: 'neutral' | 'good' | 'warn' }> = {
    L0_CLAIMED: { text: 'Claimed', tone: 'neutral' },
    L1_PHOTO_CONSISTENT: { text: 'Photo-checked', tone: 'neutral' },
    L2_CLOSE_UP: { text: 'Close-up verified', tone: 'good' },
    L3_IDENTIFIER: { text: 'Label verified', tone: 'good' },
    L4_VERIFIED_AT_PICKUP: { text: 'Verified at pickup', tone: 'good' },
  };
  const m = map[level] ?? { text: level, tone: 'neutral' as const };
  return <Badge tone={m.tone}>{m.text}</Badge>;
}
