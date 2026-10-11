'use client';
import { useQuery } from '@tanstack/react-query';
import { Badge, Button, Card, ErrorNote, Field, Spinner, formatRange, formatWeight, useApi, useAuth } from '@kabados/ui';
import { useEffect, useState } from 'react';
import { qry } from '../lib/api';

const RINGS = [
  { v: '', label: 'Any distance' },
  { v: '0', label: 'My cell' },
  { v: '1', label: 'Nearby' },
  { v: '2', label: 'Wider' },
  { v: '3', label: 'City-wide' },
];

function km(distanceM: number): string {
  if (distanceM < 1000) return `${Math.round(distanceM)} m away`;
  return `${(distanceM / 1000).toFixed(1)} km away`;
}

export default function Feed() {
  const { token } = useAuth();
  const api = useApi();
  const [ready, setReady] = useState(false);
  useEffect(() => setReady(true), []);

  const [categories, setCategories] = useState('');
  const [ring, setRing] = useState('');

  const feed = useQuery({
    queryKey: ['feed', categories, ring],
    enabled: ready && !!token,
    queryFn: async () => {
      const q: Record<string, unknown> = {};
      if (categories.trim()) q.categories = categories.trim();
      if (ring) q.ring = Number(ring);
      const { data, error } = await api.GET('/v1/vendor/feed', { params: qry(q) });
      if (error) throw error;
      return data;
    },
  });

  if (!ready) return <Spinner />;
  if (!token)
    return (
      <Card title="Sign in required">
        <p className="kb-muted">Sign in to browse listings near your yard.</p>
        <a href="/signin/"><Button>Sign in</Button></a>
      </Card>
    );

  return (
    <>
      <h1 className="kb-h1">Listings near you</h1>
      <Card>
        <div className="kb-grid-2">
          <Field label="Categories (comma-separated globs)">
            <input className="kb-input" value={categories} onChange={(e) => setCategories(e.target.value)} placeholder="metal.*, paper.*" />
          </Field>
          <Field label="Distance ring">
            <select className="kb-input" value={ring} onChange={(e) => setRing(e.target.value)}>
              {RINGS.map((r) => <option key={r.v} value={r.v}>{r.label}</option>)}
            </select>
          </Field>
        </div>
      </Card>

      {feed.isLoading ? <Spinner label="Finding listings…" /> : null}
      {feed.error ? <ErrorNote>Could not load the feed. Pull to refresh.</ErrorNote> : null}
      {feed.data && feed.data.length === 0 ? <p className="kb-empty">No matching listings right now. Try widening the distance or clearing filters.</p> : null}

      <ul className="kb-list">
        {feed.data?.map((item) => {
          const l = item.listing;
          const q = item.quote;
          return (
            <li key={l.id}>
              <a href={`/listing/?id=${l.id}`} style={{ textDecoration: 'none', color: 'inherit', display: 'block' }}>
                <div className="kb-between">
                  <strong>{l.title}</strong>
                  {l.contains_hazardous ? <Badge tone="hazard">Hazardous</Badge> : null}
                </div>
                <p className="kb-muted">
                  {l.approx.city} · {km(item.distance_m)} · {formatWeight(l.total_est_weight_g)}
                </p>
                {q && q.eligible ? (
                  <p className="kb-price">
                    {formatRange(q.quote_low, q.quote_high)}{' '}
                    <span className="kb-muted" style={{ fontWeight: 400 }}>· {Math.round(q.coverage_permille / 10)}% covered</span>
                  </p>
                ) : (
                  <p className="kb-muted">{q ? 'Not eligible under your rate card' : 'Publish a rate card to see your quote'}</p>
                )}
              </a>
            </li>
          );
        })}
      </ul>
    </>
  );
}
