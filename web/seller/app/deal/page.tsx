'use client';
import { Badge, Button, Card, ErrorNote, Field, Spinner, formatPaise, formatWeight, stars, useApi, useAuth } from '@kabados/ui';
import { useMutation, useQuery } from '@tanstack/react-query';
import { useEffect, useState } from 'react';

function useDealId() {
  const [id, setId] = useState<string | null>(null);
  useEffect(() => {
    setId(new URLSearchParams(window.location.search).get('id'));
  }, []);
  return id;
}

export default function DealDetail() {
  const { token } = useAuth();
  const api = useApi();
  const [ready, setReady] = useState(false);
  useEffect(() => setReady(true), []);
  const id = useDealId();

  const [err, setErr] = useState<string | null>(null);
  const [ratingStars, setRatingStars] = useState(5);
  const [ratingComment, setRatingComment] = useState('');

  const deal = useQuery({
    queryKey: ['deal', id],
    enabled: ready && !!id && !!token,
    queryFn: async () => {
      const { data, error } = await api.GET(`/v1/deals/${id}` as '/v1/deals/{id}');
      if (error) throw error;
      return data;
    },
  });

  const taxonomy = useQuery({
    queryKey: ['taxonomy'],
    enabled: ready,
    queryFn: async () => {
      const { data, error } = await api.GET('/v1/public/taxonomy');
      if (error) throw error;
      return data;
    },
  });

  const settled = deal.data?.deal.status === 'SETTLED';
  const receipt = useQuery({
    queryKey: ['receipt', id],
    enabled: ready && !!id && !!token && settled,
    queryFn: async () => {
      const { data, error } = await api.GET(`/v1/deals/${id}/receipt` as '/v1/deals/{id}/receipt');
      if (error) throw error;
      return data;
    },
  });

  const action = useMutation({
    mutationFn: async (verb: 'confirm-weights' | 'confirm-received') => {
      setErr(null);
      const { error } = await api.POST(`/v1/deals/${id}:${verb}` as '/v1/deals/{id}:confirm-weights', { body: {} });
      if (error) throw error;
    },
    onSuccess: () => deal.refetch(),
    onError: (e) => setErr(e instanceof Error ? e.message : 'Could not complete that action'),
  });

  const rate = useMutation({
    mutationFn: async () => {
      setErr(null);
      const { error } = await api.POST(`/v1/deals/${id}/ratings` as '/v1/deals/{id}/ratings', { body: { stars: ratingStars, comment: ratingComment || null } });
      if (error) throw error;
    },
    onSuccess: () => deal.refetch(),
    onError: (e) => setErr(e instanceof Error ? e.message : 'Could not submit your rating'),
  });

  if (!ready || id === null) return <Spinner />;
  if (!token)
    return (
      <Card title="Sign in">
        <a href="/signin/"><Button>Sign in</Button></a>
      </Card>
    );
  if (deal.isLoading) return <Spinner />;
  if (deal.error) return <ErrorNote>Could not load this pickup.</ErrorNote>;
  if (!deal.data) return <p className="kb-empty">Pickup not found.</p>;

  const d = deal.data;
  const status = d.deal.status;
  const amount = d.deal.final_amount ?? d.deal.agreed_total_high;
  const modeB = d.deal.payment_mode === 'B_DIRECT';
  const paid = status === 'PAID' || status === 'PAID_DIRECT';
  const cats = taxonomy.data?.categories ?? [];
  const catName = (cid: string) => cats.find((c) => c.id === cid)?.name ?? cid;

  return (
    <>
      <div className="kb-between">
        <h1 className="kb-h1">{d.listing.title}</h1>
        <Badge tone={settled ? 'good' : 'neutral'}>{status.replace(/_/g, ' ').toLowerCase()}</Badge>
      </div>
      {err ? <ErrorNote>{err}</ErrorNote> : null}

      <Card title="Summary">
        <div className="kb-between"><span className="kb-muted">{d.deal.final_amount != null ? 'Final amount' : 'Agreed (up to)'}</span><strong className="kb-price">{formatPaise(amount)}</strong></div>
        {d.counterpart_name ? <div className="kb-between"><span className="kb-muted">Vendor</span><span>{d.counterpart_name}</span></div> : null}
        <div className="kb-between"><span className="kb-muted">Payment</span><span>{modeB ? 'Direct to you' : 'Via platform'}</span></div>
        {d.deal.slot ? <div className="kb-between"><span className="kb-muted">Pickup</span><span>{d.deal.slot.date} {d.deal.slot.window}</span></div> : null}
        {d.exact_address ? (
          <div className="kb-between"><span className="kb-muted">Address</span><span style={{ textAlign: 'right' }}>{d.exact_address.text}{d.exact_address.landmark ? <><br /><span className="kb-muted">{d.exact_address.landmark}</span></> : null}</span></div>
        ) : null}
      </Card>

      {/* Handover code — shown prominently once paid */}
      {paid && d.handover_otp ? (
        <Card title="Handover code">
          <p className="kb-muted">Give this code to the vendor or transporter at pickup to release your scrap.</p>
          <p className="kb-price" style={{ fontSize: '2rem', letterSpacing: '0.3em' }}>{d.handover_otp}</p>
        </Card>
      ) : null}

      <Card title="Items & weights">
        {d.lines.length === 0 ? <p className="kb-muted">No lines yet.</p> : null}
        <ul className="kb-list">
          {d.lines.map((ln) => (
            <li key={ln.n} className="kb-between">
              <span>
                <strong>{catName(ln.category_id)}</strong>{ln.grade ? ` · ${ln.grade}` : ''}
                <br />
                <span className="kb-muted">{ln.quantity} × · {ln.weighed_g != null ? formatWeight(ln.weighed_g) : 'not weighed yet'} · {ln.status.toLowerCase()}</span>
              </span>
              {ln.amount != null ? <span className="kb-price">{formatPaise(ln.amount)}</span> : null}
            </li>
          ))}
        </ul>
      </Card>

      {/* Status-driven seller actions */}
      {status === 'WEIGHED' ? (
        <Card title="Confirm weights">
          <p className="kb-muted">The vendor has weighed your items. Confirm to lock the final amount.</p>
          <Button onClick={() => action.mutate('confirm-weights')} disabled={action.isPending}>{action.isPending ? 'Confirming…' : 'Confirm weights'}</Button>
        </Card>
      ) : null}

      {modeB && status === 'SELLER_CONFIRMED' ? (
        <Card title="Confirm you were paid">
          <p className="kb-muted">
            {d.deal.direct_payment_marked_at ? 'The vendor marked this as paid. ' : 'Once the vendor pays you directly, '}
            confirm here to get your handover code.
          </p>
          <Button onClick={() => action.mutate('confirm-received')} disabled={action.isPending}>{action.isPending ? 'Confirming…' : 'I received payment'}</Button>
        </Card>
      ) : null}

      {/* Green Receipt */}
      {settled ? (
        <Card title="Green Receipt">
          {receipt.isLoading ? <Spinner /> : null}
          {receipt.data ? (
            <>
              <div className="kb-between"><span className="kb-muted">Total weight</span><strong>{formatWeight(receipt.data.receipt.total_weight)}</strong></div>
              <div className="kb-between"><span className="kb-muted">Total amount</span><strong className="kb-price">{formatPaise(receipt.data.receipt.total_amount)}</strong></div>
              <div className="kb-between"><span className="kb-muted">CO₂e avoided</span><strong>{formatWeight(receipt.data.receipt.co2e_avoided_g)}</strong></div>
              <p className="kb-muted">{receipt.data.receipt.disclaimer}</p>
            </>
          ) : null}
        </Card>
      ) : null}

      {/* Rating */}
      <Card title="Rate this pickup">
        {d.my_rating != null ? (
          <p>You rated this {stars(d.my_rating)}</p>
        ) : (
          <>
            <Field label="Stars">
              <select className="kb-input" value={ratingStars} onChange={(e) => setRatingStars(Number(e.target.value))}>
                {[5, 4, 3, 2, 1].map((n) => (
                  <option key={n} value={n}>{stars(n)} ({n})</option>
                ))}
              </select>
            </Field>
            <Field label="Comment (optional)">
              <input className="kb-input" value={ratingComment} onChange={(e) => setRatingComment(e.target.value)} />
            </Field>
            <Button onClick={() => rate.mutate()} disabled={rate.isPending}>{rate.isPending ? 'Submitting…' : 'Submit rating'}</Button>
          </>
        )}
      </Card>
    </>
  );
}
