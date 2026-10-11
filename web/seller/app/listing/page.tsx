'use client';
import { Badge, Button, Card, ErrorNote, Field, LadderBadge, PhotoUpload, Spinner, formatRange, formatWeight, stars, useApi, useAuth } from '@kabados/ui';
import { useMutation, useQuery } from '@tanstack/react-query';
import { useRouter } from 'next/navigation';
import { useEffect, useState } from 'react';

function useListingId() {
  const [id, setId] = useState<string | null>(null);
  useEffect(() => {
    setId(new URLSearchParams(window.location.search).get('id'));
  }, []);
  return id;
}

const VERDICT: Record<string, { text: string; tone: 'good' | 'warn' | 'hazard' }> = {
  SUPPORTED: { text: 'Supported', tone: 'good' },
  INSUFFICIENT_EVIDENCE: { text: 'Needs proof', tone: 'warn' },
  INCONSISTENT: { text: 'Inconsistent', tone: 'hazard' },
};

export default function ListingDetail() {
  const { token } = useAuth();
  const api = useApi();
  const router = useRouter();
  const [ready, setReady] = useState(false);
  useEffect(() => setReady(true), []);
  const id = useListingId();

  const [err, setErr] = useState<string | null>(null);
  const [counterFor, setCounterFor] = useState<string | null>(null);
  const [counterRupees, setCounterRupees] = useState('');
  const [counterNote, setCounterNote] = useState('');

  const listing = useQuery({
    queryKey: ['listing', id],
    enabled: ready && !!id && !!token,
    queryFn: async () => {
      const { data, error } = await api.GET(`/v1/listings/${id}` as '/v1/listings/{id}');
      if (error) throw error;
      return data;
    },
  });

  const suggestion = useQuery({
    queryKey: ['suggestion', id],
    enabled: ready && !!id && !!token,
    queryFn: async () => {
      const { data, error } = await api.GET(`/v1/listings/${id}/price-suggestion` as '/v1/listings/{id}/price-suggestion');
      if (error) throw error;
      return data;
    },
  });

  const offers = useQuery({
    queryKey: ['offers', id],
    enabled: ready && !!id && !!token,
    queryFn: async () => {
      const { data, error } = await api.GET(`/v1/listings/${id}/offers` as '/v1/listings/{id}/offers');
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

  const skipProofs = useMutation({
    mutationFn: async () => {
      setErr(null);
      const { data, error } = await api.POST(`/v1/listings/${id}/proofs:skip` as '/v1/listings/{id}/proofs:skip');
      if (error) throw error;
      return data;
    },
    onSuccess: () => listing.refetch(),
    onError: (e) => setErr(e instanceof Error ? e.message : 'Could not skip proofs'),
  });

  const provideProof = useMutation({
    mutationFn: async ({ proofId, imageId }: { proofId: string; imageId: string }) => {
      setErr(null);
      const { data, error } = await api.POST(`/v1/listings/${id}/proofs` as '/v1/listings/{id}/proofs', { body: { proof_request_id: proofId, image_ids: [imageId] } });
      if (error) throw error;
      return data;
    },
    onSuccess: () => listing.refetch(),
    onError: (e) => setErr(e instanceof Error ? e.message : 'Could not submit proof'),
  });

  const accept = useMutation({
    mutationFn: async (offerId: string) => {
      setErr(null);
      const { error } = await api.POST(`/v1/offers/${offerId}:accept` as '/v1/offers/{id}:accept', { body: { slot: null } });
      if (error) throw error;
      // Accept returns no body; the deal id now lives on the refreshed listing.
      const { data, error: e2 } = await api.GET(`/v1/listings/${id}` as '/v1/listings/{id}');
      if (e2) throw e2;
      return data.deal_id ?? null;
    },
    onSuccess: (dealId) => {
      if (dealId) router.push(`/deal/?id=${dealId}`);
      else router.push('/deals/');
    },
    onError: (e) => setErr(e instanceof Error ? e.message : 'Could not accept the offer'),
  });

  const counter = useMutation({
    mutationFn: async (offerId: string) => {
      setErr(null);
      const rupees = Number(counterRupees);
      if (Number.isNaN(rupees) || rupees <= 0) throw new Error('Enter a counter amount in rupees');
      const body = { total: Math.round(rupees * 100), note: counterNote || null };
      const { error } = await api.POST(`/v1/offers/${offerId}:counter` as '/v1/offers/{id}:accept', { body: body as never });
      if (error) throw error;
    },
    onSuccess: () => {
      setCounterFor(null);
      setCounterRupees('');
      setCounterNote('');
      offers.refetch();
    },
    onError: (e) => setErr(e instanceof Error ? e.message : 'Could not send counter'),
  });

  if (!ready || id === null) return <Spinner />;
  if (!token)
    return (
      <Card title="Sign in">
        <a href="/signin/"><Button>Sign in</Button></a>
      </Card>
    );
  if (listing.isLoading) return <Spinner />;
  if (listing.error) return <ErrorNote>Could not load this listing.</ErrorNote>;
  if (!listing.data) return <p className="kb-empty">Listing not found.</p>;

  const l = listing.data;
  const cats = taxonomy.data?.categories ?? [];
  const catName = (cid: string) => cats.find((c) => c.id === cid)?.name ?? cid;
  const openProofs = l.proof_requests.filter((p) => p.status === 'OPEN');

  return (
    <>
      <div className="kb-between">
        <h1 className="kb-h1">{l.listing.title}</h1>
        <Badge>{l.listing.status.toLowerCase()}</Badge>
      </div>
      {err ? <ErrorNote>{err}</ErrorNote> : null}

      <Card title="Photos">
        {l.images.length === 0 ? <p className="kb-muted">No photos on this listing.</p> : (
          <div className="kb-row">
            {l.images.map((im) => (im.thumb_url || im.url) ? <img key={im.id} src={im.thumb_url ?? im.url ?? ''} alt="" style={{ width: 84, height: 84, objectFit: 'cover', borderRadius: 8 }} /> : null)}
          </div>
        )}
        {l.listing.status === 'DRAFT' || l.listing.status === 'NEEDS_INPUT' || l.listing.status === 'READY' ? (
          <div className="kb-row" style={{ marginTop: 10 }}>
            <PhotoUpload listingId={id} kind="WIDE" source="CAMERA" label="📷 Add photo" onUploaded={() => listing.refetch()} />
            <PhotoUpload listingId={id} kind="WIDE" source="GALLERY" label="Upload" onUploaded={() => listing.refetch()} />
          </div>
        ) : null}
      </Card>

      <Card title="Items">
        {l.items.length === 0 ? <p className="kb-muted">No items yet.</p> : null}
        <ul className="kb-list">
          {l.items.map((it) => (
            <li key={it.id} className="kb-between">
              <span>
                <strong>{catName(it.category_id)}</strong>{it.grade ? ` · ${it.grade}` : ''}
                <br />
                <span className="kb-muted">{it.quantity} × · {formatWeight(it.est_weight_g ?? 0)}</span>
              </span>
              <span className="kb-row">
                {it.hazard.is_hazardous ? <Badge tone="hazard">Hazardous</Badge> : null}
                <LadderBadge level={it.ladder} />
              </span>
            </li>
          ))}
        </ul>
      </Card>

      {l.claims.length > 0 ? (
        <Card title="Verification">
          <ul className="kb-list">
            {l.claims.map((c) => {
              const v = VERDICT[c.verdict] ?? { text: c.verdict, tone: 'warn' as const };
              return (
                <li key={c.id}>
                  <div className="kb-between">
                    <span>{c.text}</span>
                    <Badge tone={v.tone}>{v.text}</Badge>
                  </div>
                  {c.rationale ? <p className="kb-muted">{c.rationale}</p> : null}
                </li>
              );
            })}
          </ul>
          {openProofs.length > 0 ? (
            <>
              <h3 className="kb-card-title">To raise your price, add these proofs</h3>
              <ul className="kb-list">
                {openProofs.map((p) => (
                  <li key={p.id}>
                    <strong>{p.instruction}</strong>
                    <br />
                    <span className="kb-muted">{p.benefit}</span>
                    <div style={{ marginTop: 8 }}>
                      <PhotoUpload listingId={id} kind="PROOF" source="CAMERA" label="📷 Add this proof" onUploaded={(imageId) => provideProof.mutate({ proofId: p.id, imageId })} />
                    </div>
                  </li>
                ))}
              </ul>
              <p className="kb-muted">Add the proofs above, or skip and go to offers now.</p>
              <Button variant="ghost" onClick={() => skipProofs.mutate()} disabled={skipProofs.isPending}>{skipProofs.isPending ? 'Skipping…' : 'Skip proofs'}</Button>
            </>
          ) : null}
        </Card>
      ) : null}

      <Card title="Suggested price">
        {suggestion.isLoading ? <Spinner /> : null}
        {suggestion.data ? (
          suggestion.data.low != null && suggestion.data.high != null ? (
            <>
              <p className="kb-price">{formatRange(suggestion.data.low, suggestion.data.high)}</p>
              <p className="kb-muted">{suggestion.data.vendor_count} nearby vendor{suggestion.data.vendor_count === 1 ? '' : 's'} · {suggestion.data.basis.replace(/_/g, ' ').toLowerCase()}</p>
            </>
          ) : (
            <p className="kb-muted">No price data yet.</p>
          )
        ) : null}
      </Card>

      <Card title="Offers">
        {offers.isLoading ? <Spinner /> : null}
        {offers.data && offers.data.length === 0 ? <p className="kb-empty">No offers yet. Vendors nearby are reviewing your lot.</p> : null}
        <ul className="kb-list">
          {offers.data?.map((o) => {
            const active = o.status === 'ACTIVE';
            return (
              <li key={o.id}>
                <div className="kb-between">
                  <span>
                    <strong className="kb-price">{formatRange(o.total_low, o.total_high)}</strong>
                    <br />
                    <span className="kb-muted">{o.vendor_name} · {stars(o.vendor_rating)}</span>
                  </span>
                  <span style={{ textAlign: 'right' }}>
                    {o.meets_ask ? <Badge tone="good">Meets your ask</Badge> : null}
                    {!active ? <Badge>{o.status.toLowerCase()}</Badge> : null}
                  </span>
                </div>
                <p className="kb-muted">
                  {o.slot ? `Pickup ${o.slot.date} ${o.slot.window}` : 'Pickup slot to be agreed'}
                  {o.payment_mode === 'A_PLATFORM' ? ' · pays via platform' : ' · pays directly'}
                </p>
                {o.note ? <p className="kb-muted">“{o.note}”</p> : null}
                {o.seller_counter ? <p className="kb-muted">Your counter: sent</p> : null}
                {active ? (
                  <>
                    <div className="kb-row">
                      <Button onClick={() => accept.mutate(o.id)} disabled={accept.isPending}>{accept.isPending ? 'Accepting…' : 'Accept'}</Button>
                      <Button variant="ghost" onClick={() => setCounterFor(counterFor === o.id ? null : o.id)}>Counter</Button>
                    </div>
                    {counterFor === o.id ? (
                      <div>
                        <Field label="Your counter (₹)">
                          <input className="kb-input" type="number" min={1} value={counterRupees} onChange={(e) => setCounterRupees(e.target.value)} inputMode="decimal" />
                        </Field>
                        <Field label="Note (optional)">
                          <input className="kb-input" value={counterNote} onChange={(e) => setCounterNote(e.target.value)} />
                        </Field>
                        <Button onClick={() => counter.mutate(o.id)} disabled={counter.isPending}>{counter.isPending ? 'Sending…' : 'Send counter'}</Button>
                      </div>
                    ) : null}
                  </>
                ) : null}
              </li>
            );
          })}
        </ul>
      </Card>
    </>
  );
}
