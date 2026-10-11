'use client';
import { useAuth } from '@kabados/ui';
export function Header() {
  const { token, signOut } = useAuth();
  return (
    <header className="kb-header"><div className="kb-header-inner">
      <a className="kb-brand" href="/">♻ KabadOS Ops</a>
      <nav className="kb-nav">
        <a href="/kyc/">KYC</a><a href="/disputes/">Disputes</a><a href="/ai-reviews/">AI review</a>
        <a href="/rate-index/">Rate index</a><a href="/config/">Config</a>
        {token ? <a href="#" onClick={(e) => { e.preventDefault(); signOut(); }}>Sign out</a> : <a href="/signin/">Sign in</a>}
      </nav>
    </div></header>
  );
}
