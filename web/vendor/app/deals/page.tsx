'use client';
import { useQuery } from '@tanstack/react-query';
import { Badge, Button, Card, ErrorNote, Spinner, formatDate, formatPaise, useApi, useAuth } from '@kabados/ui';
import { useEffect, useState } from 'react';

// `index` is typed as an opaque object in the generated schema; at runtime it
// carries the deal index key (PLAN deal index), including the id we link with.
type DealIndex = { deal_id: string; listing_id: string; created_at: string };

const OPEN = new Set(['CLOSED', 'SETTLED', 'CANCELLED']);
const TONE: Record<string, 'neutral' | 'good' | 'warn' | 'hazard'> = {
  SELLER_NO_SHOW: 'hazard',
  PICKUP_NO_SHOW: 'hazard',
  DISPUTED: 'hazard',
  CANCELLED: 'neutral',
  CLOSED: 'neutral',
  SETTLED: 'good',
  DELIVERED: 'good',
  PAID: 'good',
  PAID_DIRECT: 'good',
};

export default function Deals() {
  const { token } = useAuth();
  const api = useApi();
  const [ready, setReady] = useState(false);
  useEffect(() => setReady(true), []);

  const deals = useQuery({
    queryKey: ['deals'],
    enabled: ready && !!token,
    queryFn: async () => {
      const { data, error } = await api.GET('/v1/deals');
      if (error) throw error;
      return data;
    },
  });

  if (!ready) return <Spinner />;
  if (!token)
    return (
      <Card title="Sign in required">
        <a href="/signin/"><Button>Sign in</Button></a>
      </Card>
    );

  const rows = deals.data ?? [];
  const open = rows.filter((d) => !OPEN.has(d.status));
  const done = rows.filter((d) => OPEN.has(d.status));

  const render = (d: (typeof rows)[number]) => {
    const idx = d.index as unknown as DealIndex;
    const amount = d.final_amount ?? d.agreed_total_high;
    return (
      <li key={idx.deal_id}>
        <a href={`/deal/?id=${idx.deal_id}`} style={{ textDecoration: 'none', color: 'inherit', display: 'block' }}>
          <div className="kb-between">
            <strong>{d.title || 'Pickup'}</strong>
            <Badge tone={TONE[d.status] ?? 'warn'}>{d.status.replaceAll('_', ' ').toLowerCase()}</Badge>
          </div>
          <p className="kb-muted">
            {d.final_amount ? 'Final ' : 'Up to '}{formatPaise(amount)}
            {d.slot ? ` · ${formatDate(d.slot.date)} ${d.slot.window}` : ''}
          </p>
        </a>
      </li>
    );
  };

  return (
    <>
      <h1 className="kb-h1">Deals</h1>
      {deals.isLoading ? <Spinner label="Loading deals…" /> : null}
      {deals.error ? <ErrorNote>Could not load your deals.</ErrorNote> : null}
      {deals.data && rows.length === 0 ? <p className="kb-empty">No deals yet. Accepted offers turn into deals here.</p> : null}

      {open.length ? <><h2 className="kb-card-title">In progress</h2><ul className="kb-list">{open.map(render)}</ul></> : null}
      {done.length ? <><h2 className="kb-card-title">Completed</h2><ul className="kb-list">{done.map(render)}</ul></> : null}
    </>
  );
}
