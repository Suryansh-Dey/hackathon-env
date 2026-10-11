'use client';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Badge, Button, Card, ErrorNote, Field, Spinner, formatDate, formatPaise, formatWeight, useApi, useAuth } from '@kabados/ui';
import { useEffect, useState } from 'react';
import { pth, type DealActionPath, type NoBodyIdPath } from '../lib/api';

type PaymentRequest = {
  order: { id: string; amount: number; simulated: boolean };
  seller_amount: number;
  transporter_fare: number;
  platform_fee: number;
};

export default function DealDetail() {
  const { token } = useAuth();
  const api = useApi();
  const qc = useQueryClient();
  const [ready, setReady] = useState(false);
  const [id, setId] = useState<string | null>(null);
  useEffect(() => {
    setReady(true);
    try {
      setId(new URLSearchParams(window.location.search).get('id'));
    } catch {
      /* SSR */
    }
  }, []);
  const enabled = ready && !!token && !!id;

  const deal = useQuery({
    queryKey: ['deal', id],
    enabled,
    queryFn: async () => {
      const { data, error } = await api.GET('/v1/deals/{id}', { params: pth({ id: id! }) });
      if (error) throw error;
      return data;
    },
  });

  const [weights, setWeights] = useState<Record<number, string>>({});
  const [otp, setOtp] = useState('');
  const [payReq, setPayReq] = useState<PaymentRequest | null>(null);
  const invalidate = () => qc.invalidateQueries({ queryKey: ['deal', id] });

  // One deal-action helper: every `:action` variant casts to the typed path.
  const action = useMutation({
    mutationFn: async (vars: { name: string; body?: { otp?: string } }) => {
      const { error } = await api.POST(`/v1/deals/{id}:${vars.name}` as DealActionPath, {
        params: pth({ id: id! }),
        body: vars.body ?? {},
      });
      if (error) throw error;
    },
    onSuccess: invalidate,
  });

  const weigh = useMutation({
    mutationFn: async (vars: { line_n: number; reading_g: number }) => {
      const { error } = await api.POST('/v1/deals/{id}/weighings', { params: pth({ id: id! }), body: vars });
      if (error) throw error;
    },
    onSuccess: invalidate,
  });

  const requestPayment = useMutation({
    mutationFn: async () => {
      const { data, error } = await api.POST('/v1/deals/{id}/payment-request', { params: pth({ id: id! }) });
      if (error) throw error;
      return data as unknown as PaymentRequest;
    },
    onSuccess: (data) => {
      setPayReq(data);
      invalidate();
    },
  });

  const simulate = useMutation({
    mutationFn: async (orderId: string) => {
      // Dev/mock capture: this route is off the typed contract, so we cast the
      // path onto a no-body id route; the auth + idempotency middleware still runs.
      const { error } = await api.POST(`/v1/payments/{id}:simulate-capture` as NoBodyIdPath, { params: pth({ id: orderId }) });
      if (error) throw error;
    },
    onSuccess: invalidate,
  });

  if (!ready) return <Spinner />;
  if (!token)
    return <Card title="Sign in required"><a href="/signin/"><Button>Sign in</Button></a></Card>;
  if (!id) return <Card title="Deal not found"><a href="/deals/"><Button>Back to deals</Button></a></Card>;
  if (deal.isLoading) return <Spinner label="Loading deal…" />;
  if (deal.error || !deal.data) return <ErrorNote>Could not load this deal.</ErrorNote>;

  const v = deal.data;
  const d = v.deal;
  const status = d.status;
  const isSelf = d.pickup_method === 'SELF_PICKUP';
  const isModeA = d.payment_mode === 'A_PLATFORM';
  const orderId = payReq?.order.id ?? d.payment_order_id ?? null;

  const canArrive = isSelf && (status === 'PICKUP_SCHEDULED' || status === 'CREATED');
  const canWeigh = status === 'AT_PICKUP' || status === 'WEIGHED' || status === 'LINE_REGRADED';
  const canRequestPayment = isModeA && status === 'SELLER_CONFIRMED';
  const canSimulate = isModeA && (status === 'AWAITING_PAYMENT' || !!payReq) && !!orderId;
  const canMarkPaidDirect = !isModeA && status === 'SELLER_CONFIRMED';
  const canHandover = status === 'PAID' || status === 'PAID_DIRECT';
  const canConfirmDelivery = status === 'HANDED_OVER' || status === 'IN_TRANSIT';

  const actErr = action.error || weigh.error || requestPayment.error || simulate.error;

  return (
    <>
      <p><a href="/deals/" className="kb-muted">← Back to deals</a></p>
      <div className="kb-between">
        <h1 className="kb-h1">{v.listing.title || 'Pickup'}</h1>
        <Badge tone="warn">{status.replaceAll('_', ' ').toLowerCase()}</Badge>
      </div>
      <p className="kb-muted">
        {isSelf ? 'Self-pickup' : 'Transporter'} · {isModeA ? 'Platform-mediated (Mode A)' : 'Direct (Mode B)'}
        {v.counterpart_name ? ` · Seller: ${v.counterpart_name}` : ''}
        {d.slot ? ` · ${formatDate(d.slot.date)} ${d.slot.window}` : ''}
      </p>

      <Card title="Amount">
        <p className="kb-price">{d.final_amount ? formatPaise(d.final_amount) : `${formatPaise(d.agreed_total_low)} – ${formatPaise(d.agreed_total_high)}`}</p>
        <p className="kb-muted">
          {d.final_amount ? 'Final weighed amount' : 'Agreed estimate (final set after weighing)'}
          {d.transporter_fare ? ` · transporter fare ${formatPaise(d.transporter_fare)}` : ''} · platform fee {formatPaise(d.platform_fee)}
        </p>
      </Card>

      {v.exact_address ? (
        <Card title="Pickup address">
          <p>{v.exact_address.text}</p>
          {v.exact_address.landmark ? <p className="kb-muted">Landmark: {v.exact_address.landmark}</p> : null}
        </Card>
      ) : null}

      <Card title="Items & weighing">
        <ul className="kb-list">
          {v.lines.map((line) => {
            const done = v.weighings.find((w) => w.line_n === line.n);
            return (
              <li key={line.n}>
                <div className="kb-between">
                  <span>
                    <strong>{line.category_id}</strong>{line.grade ? ` · ${line.grade}` : ''}
                    <br />
                    <span className="kb-muted">
                      qty {line.quantity} · {line.status.toLowerCase()}
                      {line.weighed_g != null ? ` · weighed ${formatWeight(line.weighed_g)}` : ''}
                      {line.amount != null ? ` · ${formatPaise(line.amount)}` : ''}
                    </span>
                  </span>
                </div>
                {canWeigh && line.weighed_g == null ? (
                  <div className="kb-row" style={{ gap: 8, marginTop: 8 }}>
                    <input
                      className="kb-input"
                      inputMode="numeric"
                      placeholder="grams on scale"
                      value={weights[line.n] ?? ''}
                      onChange={(e) => setWeights((w) => ({ ...w, [line.n]: e.target.value }))}
                      style={{ maxWidth: 160 }}
                    />
                    <Button
                      variant="ghost"
                      disabled={weigh.isPending || !weights[line.n]}
                      onClick={() => {
                        const g = Number(weights[line.n]);
                        if (!Number.isNaN(g) && g > 0) weigh.mutate({ line_n: line.n, reading_g: g });
                      }}
                    >
                      {done ? 'Re-record' : 'Record weight'}
                    </Button>
                  </div>
                ) : null}
              </li>
            );
          })}
        </ul>
        {canWeigh ? <p className="kb-muted">Record each line’s scale reading; the seller confirms the final amount.</p> : null}
      </Card>

      <Card title="Next step">
        {actErr ? <ErrorNote>That action didn’t go through. Check the deal is at the right stage and try again.</ErrorNote> : null}

        {canArrive ? (
          <p><Button disabled={action.isPending} onClick={() => action.mutate({ name: 'arrive' })}>Mark arrived at pickup</Button></p>
        ) : null}

        {canRequestPayment ? (
          <p><Button disabled={requestPayment.isPending} onClick={() => requestPayment.mutate()}>Request payment from seller</Button></p>
        ) : null}

        {payReq ? (
          <p className="kb-muted">
            Payment requested · seller {formatPaise(payReq.seller_amount)} · fee {formatPaise(payReq.platform_fee)}
            {payReq.transporter_fare ? ` · transporter ${formatPaise(payReq.transporter_fare)}` : ''}
          </p>
        ) : null}

        {canSimulate && orderId ? (
          <p>
            <Button variant="ghost" disabled={simulate.isPending} onClick={() => simulate.mutate(orderId)}>
              {simulate.isPending ? 'Simulating…' : 'Simulate payment (dev)'}
            </Button>
          </p>
        ) : null}

        {canMarkPaidDirect ? (
          <p><Button disabled={action.isPending} onClick={() => action.mutate({ name: 'mark-paid-direct' })}>Mark paid directly (Mode B)</Button></p>
        ) : null}

        {canHandover ? (
          <>
            <Field label="Handover OTP (from seller)">
              <input className="kb-input" value={otp} onChange={(e) => setOtp(e.target.value)} inputMode="numeric" placeholder="6-digit code" style={{ maxWidth: 200 }} />
            </Field>
            <Button disabled={action.isPending || !otp.trim()} onClick={() => action.mutate({ name: 'handover', body: { otp: otp.trim() } })}>Confirm handover</Button>
          </>
        ) : null}

        {canConfirmDelivery ? (
          <p><Button disabled={action.isPending} onClick={() => action.mutate({ name: 'confirm-delivery' })}>Confirm delivery at yard</Button></p>
        ) : null}

        {!canArrive && !canWeigh && !canRequestPayment && !canSimulate && !canMarkPaidDirect && !canHandover && !canConfirmDelivery ? (
          <p className="kb-muted">No action needed from you right now.</p>
        ) : null}
      </Card>
    </>
  );
}
