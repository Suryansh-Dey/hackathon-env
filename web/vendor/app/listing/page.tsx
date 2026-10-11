'use client';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Badge, Button, Card, ErrorNote, Field, LadderBadge, Spinner, formatRange, formatWeight, useApi, useAuth } from '@kabados/ui';
import { useEffect, useState } from 'react';
import { msg, pth } from '../lib/api';

type PickupMethod = 'SELF_PICKUP' | 'TRANSPORTER';
type PaymentMode = 'A_PLATFORM' | 'B_DIRECT';

export default function ListingDetail() {
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

  const listing = useQuery({
    queryKey: ['listing', id],
    enabled,
    queryFn: async () => {
      const { data, error } = await api.GET('/v1/listings/{id}', { params: pth({ id: id! }) });
      if (error) throw error;
      return data;
    },
  });
  const quote = useQuery({
    queryKey: ['my-quote', id],
    enabled,
    queryFn: async () => {
      const { data, error } = await api.GET('/v1/listings/{id}/my-quote', { params: pth({ id: id! }) });
      if (error) throw error;
      return data;
    },
  });

  const [pickup, setPickup] = useState<PickupMethod>('SELF_PICKUP');
  const [payment, setPayment] = useState<PaymentMode>('A_PLATFORM');
  const [validHours, setValidHours] = useState('24');
  const [slotDate, setSlotDate] = useState('');
  const [slotWindow, setSlotWindow] = useState('');
  const [note, setNote] = useState('');

  const offer = useMutation({
    mutationFn: async () => {
      const body: {
        pickup_method: PickupMethod;
        payment_mode: PaymentMode;
        valid_hours?: number;
        note?: string;
        slot?: { date: string; window: string };
      } = { pickup_method: pickup, payment_mode: payment };
      const vh = Number(validHours);
      if (!Number.isNaN(vh) && vh > 0) body.valid_hours = vh;
      if (note.trim()) body.note = note.trim();
      if (slotDate && slotWindow.trim()) body.slot = { date: slotDate, window: slotWindow.trim() };
      const { data, error } = await api.POST('/v1/listings/{id}/offers', { params: pth({ id: id! }), body });
      if (error) throw error;
      return data;
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['vendor-offers-all'] });
    },
  });

  if (!ready) return <Spinner />;
  if (!token)
    return (
      <Card title="Sign in required">
        <a href="/signin/"><Button>Sign in</Button></a>
      </Card>
    );
  if (!id) return <Card title="Listing not found"><p className="kb-muted">No listing id in the link.</p><a href="/feed/"><Button>Back to listings</Button></a></Card>;
  if (listing.isLoading) return <Spinner label="Loading listing…" />;
  if (listing.error || !listing.data) return <ErrorNote>{msg(listing.error, 'Could not load this listing.')}</ErrorNote>;

  const v = listing.data;
  const l = v.listing;
  const q = quote.data;
  const photoCount = v.images.length;

  return (
    <>
      <p><a href="/feed/" className="kb-muted">← Back to listings</a></p>
      <div className="kb-between">
        <h1 className="kb-h1">{l.title}</h1>
        {l.contains_hazardous ? <Badge tone="hazard">Hazardous</Badge> : null}
      </div>
      <p className="kb-muted">
        {l.approx.city} · {formatWeight(l.total_est_weight_g)} · <LadderBadge level={l.min_ladder} />
      </p>
      {v.version.description ? <p>{v.version.description}</p> : null}
      {v.version.asking_price ? <p className="kb-muted">Seller asking around {formatRange(v.version.asking_price, v.version.asking_price)}</p> : null}

      <Card title={`Items (${v.items.length})`}>
        <ul className="kb-list">
          {v.items.map((it) => (
            <li key={it.id}>
              <div className="kb-between">
                <span>
                  <strong>{it.category_id}</strong>{it.grade ? ` · ${it.grade}` : ''}
                  <br />
                  <span className="kb-muted">
                    {it.quantity} {it.unit === 'KG' ? '' : '×'} {it.est_weight_g ? formatWeight(it.est_weight_g) : ''}
                    {it.hazard.is_hazardous ? ' · hazardous' : ''}
                  </span>
                </span>
                <LadderBadge level={it.ladder} />
              </div>
              {it.note ? <p className="kb-muted">{it.note}</p> : null}
            </li>
          ))}
        </ul>
        <p className="kb-muted">{photoCount} photo{photoCount === 1 ? '' : 's'} attached · seller honesty {Math.round(v.seller_honesty * 100)}%</p>
      </Card>

      <Card title="Your rate-card quote">
        {quote.isLoading ? <Spinner label="Pricing…" /> : null}
        {quote.error ? <ErrorNote>{msg(quote.error, 'No quote — you may not have a published rate card, or this lot isn’t covered.')}</ErrorNote> : null}
        {q ? (
          q.eligible ? (
            <>
              <p className="kb-price">{formatRange(q.quote_low, q.quote_high)}</p>
              <p className="kb-muted">
                {Math.round(q.coverage_permille / 10)}% of items covered · {q.covered_item_ids.length}/{v.items.length} items ·{' '}
                {(q.distance_m / 1000).toFixed(1)} km · rate card v{q.rule_set_version}
              </p>
              {q.explanation_codes.length ? (
                <p className="kb-muted">{q.explanation_codes.map((c) => c.replaceAll('_', ' ').toLowerCase()).join(', ')}</p>
              ) : null}
            </>
          ) : (
            <p className="kb-muted">This lot isn’t eligible under your current rate card. You can still send a manual offer below.</p>
          )
        ) : null}
      </Card>

      <Card title="Make an offer">
        <div className="kb-grid-2">
          <Field label="Pickup method">
            <select className="kb-input" value={pickup} onChange={(e) => setPickup(e.target.value as PickupMethod)}>
              <option value="SELF_PICKUP">I’ll pick up myself</option>
              <option value="TRANSPORTER">Use a transporter</option>
            </select>
          </Field>
          <Field label="Payment mode">
            <select className="kb-input" value={payment} onChange={(e) => setPayment(e.target.value as PaymentMode)}>
              <option value="A_PLATFORM">Platform-mediated (Mode A)</option>
              <option value="B_DIRECT">Direct to seller (Mode B)</option>
            </select>
          </Field>
        </div>
        <div className="kb-grid-2">
          <Field label="Offer valid for (hours)">
            <input className="kb-input" value={validHours} onChange={(e) => setValidHours(e.target.value)} inputMode="numeric" placeholder="24" />
          </Field>
          <Field label="Proposed slot date (optional)">
            <input className="kb-input" type="date" value={slotDate} onChange={(e) => setSlotDate(e.target.value)} />
          </Field>
        </div>
        <Field label="Proposed time window (optional, HH:MM-HH:MM)">
          <input className="kb-input" value={slotWindow} onChange={(e) => setSlotWindow(e.target.value)} placeholder="10:00-13:00" />
        </Field>
        <Field label="Note to seller (optional)">
          <input className="kb-input" value={note} onChange={(e) => setNote(e.target.value)} placeholder="e.g. Can collect today" />
        </Field>
        {offer.error ? <ErrorNote>{msg(offer.error, 'Could not send the offer. Check your rate card is published and the lot is eligible.')}</ErrorNote> : null}
        {offer.isSuccess ? <p className="kb-muted">Offer sent. Track it under Offers.</p> : null}
        <Button onClick={() => offer.mutate()} disabled={offer.isPending || offer.isSuccess}>
          {offer.isPending ? 'Sending…' : offer.isSuccess ? 'Offer sent' : 'Send offer'}
        </Button>
      </Card>
    </>
  );
}
