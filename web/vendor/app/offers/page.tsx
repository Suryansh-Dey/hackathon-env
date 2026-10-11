'use client';
import { useQuery } from '@tanstack/react-query';
import { Badge, Button, Card, ErrorNote, Field, Spinner, formatDateTime, formatRange, useApi, useAuth } from '@kabados/ui';
import { useEffect, useState } from 'react';
import { msg, qry } from '../lib/api';

const STATUSES = ['ALL', 'ACTIVE', 'ACCEPTED', 'REJECTED', 'WITHDRAWN', 'EXPIRED', 'SUPERSEDED'] as const;
type Filter = (typeof STATUSES)[number];

const TONE: Record<string, 'neutral' | 'good' | 'warn'> = {
  ACTIVE: 'warn',
  ACCEPTED: 'good',
  REJECTED: 'neutral',
  WITHDRAWN: 'neutral',
  EXPIRED: 'neutral',
  SUPERSEDED: 'neutral',
};

export default function Offers() {
  const { token } = useAuth();
  const api = useApi();
  const [ready, setReady] = useState(false);
  useEffect(() => setReady(true), []);
  const [filter, setFilter] = useState<Filter>('ALL');

  const offers = useQuery({
    queryKey: ['vendor-offers-all'],
    enabled: ready && !!token,
    queryFn: async () => {
      const { data, error } = await api.GET('/v1/vendor/offers', { params: qry({}) });
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

  const rows = (offers.data ?? []).filter((o) => filter === 'ALL' || o.status === filter);

  return (
    <>
      <h1 className="kb-h1">Your offers</h1>
      <Card>
        <Field label="Status">
          <select className="kb-input" value={filter} onChange={(e) => setFilter(e.target.value as Filter)}>
            {STATUSES.map((s) => <option key={s} value={s}>{s === 'ALL' ? 'All offers' : s.toLowerCase()}</option>)}
          </select>
        </Field>
      </Card>

      {offers.isLoading ? <Spinner label="Loading offers…" /> : null}
      {offers.error ? <ErrorNote>{msg(offers.error, 'Could not load your offers.')}</ErrorNote> : null}
      {offers.data && rows.length === 0 ? <p className="kb-empty">No offers in this view.</p> : null}

      <ul className="kb-list">
        {rows.map((o) => (
          <li key={o.id}>
            <a href={`/listing/?id=${o.listing_id}`} style={{ textDecoration: 'none', color: 'inherit', display: 'block' }}>
              <div className="kb-between">
                <strong>{formatRange(o.total_low, o.total_high)}</strong>
                <Badge tone={TONE[o.status] ?? 'neutral'}>{o.status.toLowerCase()}</Badge>
              </div>
              <p className="kb-muted">
                {o.pickup_method === 'SELF_PICKUP' ? 'Self-pickup' : 'Transporter'} ·{' '}
                {o.payment_mode === 'A_PLATFORM' ? 'Mode A' : 'Mode B'} · sent {formatDateTime(o.created_at)}
              </p>
              {o.status === 'ACTIVE' ? <p className="kb-muted">Valid until {formatDateTime(o.valid_until)}</p> : null}
              {o.seller_counter ? <p className="kb-muted">Seller countered: {formatRange(o.seller_counter.total, o.seller_counter.total)}</p> : null}
            </a>
          </li>
        ))}
      </ul>
    </>
  );
}
