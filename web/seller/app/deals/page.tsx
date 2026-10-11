'use client';
import { Badge, Button, Card, ErrorNote, Spinner, formatPaise, useApi, useAuth } from '@kabados/ui';
import { useQuery } from '@tanstack/react-query';
import { useEffect, useState } from 'react';

function dealId(index: unknown): string | null {
  // DealSummary.index is a DealIndex (opaque in the schema) carrying the deal id.
  if (index && typeof index === 'object' && 'deal_id' in index) {
    const v = (index as { deal_id?: unknown }).deal_id;
    if (typeof v === 'string') return v;
  }
  return null;
}

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
      <Card title="Sign in">
        <p className="kb-muted">Sign in to see your pickups.</p>
        <a href="/signin/"><Button>Sign in</Button></a>
      </Card>
    );

  return (
    <>
      <h1 className="kb-h1">Pickups</h1>
      {deals.isLoading ? <Spinner /> : null}
      {deals.error ? <ErrorNote>Could not load your pickups.</ErrorNote> : null}
      {deals.data && deals.data.length === 0 ? <p className="kb-empty">No pickups yet. Accept an offer to start one.</p> : null}
      <ul className="kb-list">
        {deals.data?.map((d, i) => {
          const id = dealId(d.index);
          const amount = d.final_amount ?? d.agreed_total_high;
          const inner = (
            <div className="kb-between">
              <span>
                <strong>{d.title}</strong>
                <br />
                <span className="kb-muted">{d.status.replace(/_/g, ' ').toLowerCase()}{d.slot ? ` · ${d.slot.date} ${d.slot.window}` : ''}</span>
              </span>
              <span className="kb-price">{formatPaise(amount)}</span>
            </div>
          );
          return (
            <li key={id ?? i}>
              {id ? (
                <a href={`/deal/?id=${id}`} style={{ textDecoration: 'none', color: 'inherit' }}>{inner}</a>
              ) : (
                inner
              )}
            </li>
          );
        })}
      </ul>
    </>
  );
}
