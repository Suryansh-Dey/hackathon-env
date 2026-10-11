'use client';
import { useQuery } from '@tanstack/react-query';
import { Badge, Button, Card, ErrorNote, Spinner, formatRange, formatWeight, useApi, useAuth } from '@kabados/ui';
import { useEffect, useState } from 'react';
import { msg } from '../lib/api';

function km(distanceM: number): string {
  if (distanceM < 1000) return `${Math.round(distanceM)} m away`;
  return `${(distanceM / 1000).toFixed(1)} km away`;
}

function firstLookLeft(until?: string | null): string | null {
  if (!until) return null;
  const ms = new Date(until).getTime() - Date.now();
  if (ms <= 0) return null;
  const mins = Math.round(ms / 60000);
  return mins >= 60 ? `${Math.round(mins / 60)}h first-look left` : `${mins}m first-look left`;
}

export default function Matches() {
  const { token } = useAuth();
  const api = useApi();
  const [ready, setReady] = useState(false);
  useEffect(() => setReady(true), []);

  const matches = useQuery({
    queryKey: ['matches'],
    enabled: ready && !!token,
    queryFn: async () => {
      const { data, error } = await api.GET('/v1/vendor/matches');
      if (error) throw error;
      return data;
    },
  });

  if (!ready) return <Spinner />;
  if (!token)
    return (
      <Card title="Sign in required">
        <p className="kb-muted">Sign in to see listings matched to your rate card.</p>
        <a href="/signin/"><Button>Sign in</Button></a>
      </Card>
    );

  return (
    <>
      <h1 className="kb-h1">First-look leads</h1>
      <p className="kb-muted">Listings matched to your rate card. Act during first-look to offer before the wider market sees them.</p>

      {matches.isLoading ? <Spinner label="Loading leads…" /> : null}
      {matches.error ? <ErrorNote>{msg(matches.error, 'Could not load your matches.')}</ErrorNote> : null}
      {matches.data && matches.data.length === 0 ? <p className="kb-empty">No leads yet. Keep your rate card broad to match more listings.</p> : null}

      <ul className="kb-list">
        {matches.data?.map((m) => {
          const l = m.listing;
          const lead = m.lead;
          const fl = firstLookLeft(lead.first_look_until);
          return (
            <li key={l.id}>
              <a href={`/listing/?id=${l.id}`} style={{ textDecoration: 'none', color: 'inherit', display: 'block' }}>
                <div className="kb-between">
                  <strong>{l.title}</strong>
                  {fl ? <Badge tone="warn">{fl}</Badge> : null}
                </div>
                <p className="kb-price">
                  {formatRange(lead.quote_low, lead.quote_high)}{' '}
                  <span className="kb-muted" style={{ fontWeight: 400 }}>· {Math.round(lead.coverage_permille / 10)}% covered</span>
                </p>
                <p className="kb-muted">
                  {l.approx.city} · {km(lead.distance_m)} · {formatWeight(l.total_est_weight_g)}
                </p>
              </a>
            </li>
          );
        })}
      </ul>
    </>
  );
}
