'use client';
import { useAuth } from '@kabados/ui';
export function Header() {
  const { token, signOut } = useAuth();
  return (
    <header className="kb-header">
      <div className="kb-header-inner">
        <a className="kb-brand" href="/">♻ KabadOS</a>
        <nav className="kb-nav">
          <a href="/">My listings</a>
          <a href="/new/">Sell scrap</a>
          <a href="/deals/">Pickups</a>
          {token ? <a href="/account/">Account</a> : null}
          {token ? <a href="#" onClick={(e) => { e.preventDefault(); signOut(); }}>Sign out</a> : <a href="/signin/">Sign in</a>}
        </nav>
      </div>
    </header>
  );
}
