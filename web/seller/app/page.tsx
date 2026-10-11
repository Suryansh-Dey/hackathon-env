'use client';
import { useQuery } from '@tanstack/react-query';
import { Button, Card, ErrorNote, Spinner, formatWeight, problemMessage, useApi, useAuth } from '@kabados/ui';
import { useEffect, useState } from 'react';
import { ProfileGate } from './profile';

export default function Home() {
  const { token } = useAuth();
  const api = useApi();
  const [ready, setReady] = useState(false);
  useEffect(() => setReady(true), []);

  const listings = useQuery({
    queryKey: ['my-listings'],
    enabled: ready && !!token,
    queryFn: async () => {
      const { data, error } = await api.GET('/v1/listings');
      if (error) throw error;
      return data;
    },
  });

  if (!ready) return <Spinner />;
  if (!token)
    return (
      <Card title="Welcome to KabadOS">
        <p className="kb-muted">List your scrap, get AI-verified item estimates, compare real offers from nearby vendors, and get paid on pickup.</p>
        <p><a href="/signin/"><Button>Get started</Button></a></p>
      </Card>
    );

  return (
    <ProfileGate>
      <div className="kb-between">
        <h1 className="kb-h1">My listings</h1>
        <a href="/new/"><Button>Sell scrap</Button></a>
      </div>
      {listings.isLoading ? <Spinner /> : null}
      {listings.error ? <ErrorNote>{problemMessage(listings.error, 'Could not load your listings')}</ErrorNote> : null}
      {listings.data && listings.data.length === 0 ? <p className="kb-empty">No listings yet. Tap “Sell scrap” to start.</p> : null}
      <ul className="kb-list">
        {listings.data?.map((l) => (
          <li key={l.id}>
            <a href={`/listing/?id=${l.id}`} className="kb-between" style={{ textDecoration: 'none', color: 'inherit' }}>
              <span>
                <strong>{l.title}</strong>
                <br />
                <span className="kb-muted">{formatWeight(l.total_est_weight_g)} · {l.status.toLowerCase()}</span>
              </span>
              <span className="kb-badge">{l.categories.length} items</span>
            </a>
          </li>
        ))}
      </ul>
    </ProfileGate>
  );
}
