'use client';
import { useAuth } from '@kabados/ui';
export function Header() {
  const { token, signOut } = useAuth();
  return (
    <header className="kb-header"><div className="kb-header-inner">
      <a className="kb-brand" href="/">♻ KabadOS Vendor</a>
      <nav className="kb-nav">
        <a href="/feed/">Listings</a><a href="/matches/">Matches</a><a href="/rate-card/">Rate card</a>
        <a href="/offers/">Offers</a><a href="/deals/">Deals</a><a href="/transporters/">Transporters</a>
        {token ? <a href="#" onClick={(e) => { e.preventDefault(); signOut(); }}>Sign out</a> : <a href="/signin/">Sign in</a>}
      </nav>
    </div></header>
  );
}
