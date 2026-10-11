'use client';
import { Badge, Button, Card, ErrorNote, Field, LadderBadge, PhotoUpload, Spinner, formatRange, formatWeight, useApi, useAuth } from '@kabados/ui';
import { useMutation, useQuery } from '@tanstack/react-query';
import { useRouter } from 'next/navigation';
import { useEffect, useState } from 'react';
import type { paths } from '@kabados/api-client';

type ItemRow = { category_id: string; grade: string; quantity: number; est_weight_g: number };

export default function NewListing() {
  const { token } = useAuth();
  const api = useApi();
  const router = useRouter();
  const [ready, setReady] = useState(false);
  useEffect(() => setReady(true), []);

  // Draft the listing is attached to once created (step 2 onward).
  const [listingId, setListingId] = useState<string | null>(null);

  // Step 1: address
  const [addressId, setAddressId] = useState<string>('');
  const [addNew, setAddNew] = useState(false);
  const [aLabel, setALabel] = useState('Home');
  const [aText, setAText] = useState('');
  const [aLandmark, setALandmark] = useState('');
  const [aCity, setACity] = useState('');
  const [aLatLng, setALatLng] = useState('');

  // Step 2: description
  const [description, setDescription] = useState('');

  // Step 3: items
  const [rows, setRows] = useState<ItemRow[]>([]);

  const [err, setErr] = useState<string | null>(null);

  const addresses = useQuery({
    queryKey: ['addresses'],
    enabled: ready && !!token,
    queryFn: async () => {
      const { data, error } = await api.GET('/v1/me/addresses');
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

  // The live listing once a draft exists (for analysis/claims/price).
  const listing = useQuery({
    queryKey: ['listing', listingId],
    enabled: ready && !!listingId,
    refetchInterval: (q) => {
      const s = q.state.data?.version.analysis.status;
      return s === 'QUEUED' || s === 'RUNNING' ? 2000 : false;
    },
    queryFn: async () => {
      const { data, error } = await api.GET(`/v1/listings/${listingId}` as '/v1/listings/{id}');
      if (error) throw error;
      return data;
    },
  });

  const suggestion = useQuery({
    queryKey: ['suggestion', listingId],
    enabled: ready && !!listingId,
    queryFn: async () => {
      const { data, error } = await api.GET(`/v1/listings/${listingId}/price-suggestion` as '/v1/listings/{id}/price-suggestion');
      if (error) throw error;
      return data;
    },
  });

  const addAddress = useMutation({
    mutationFn: async () => {
      const parts = aLatLng.split(',').map((s) => Number(s.trim()));
      if (parts.length !== 2 || parts.some((n) => Number.isNaN(n))) throw new Error('Enter the pin as "lat, lng", e.g. 19.07, 72.87');
      const body: paths['/v1/me/addresses']['post']['requestBody']['content']['application/json'] = {
        label: aLabel,
        text: aText,
        landmark: aLandmark || null,
        city: aCity || null,
        location: { lat: parts[0], lng: parts[1] },
      };
      const { data, error } = await api.POST('/v1/me/addresses', { body });
      if (error) throw error;
      return data;
    },
    onSuccess: (data) => {
      setAddNew(false);
      setAddressId(data.id);
      addresses.refetch();
    },
    onError: (e) => setErr(e instanceof Error ? e.message : 'Could not save the address'),
  });

  const createDraft = useMutation({
    mutationFn: async () => {
      setErr(null);
      if (!addressId) throw new Error('Pick a pickup address first');
      const { data, error } = await api.POST('/v1/listings', { body: { address_id: addressId, description: description || null } });
      if (error) throw error;
      return data;
    },
    onSuccess: (data) => setListingId(data.listing.id),
    onError: (e) => setErr(e instanceof Error ? e.message : 'Could not create the listing'),
  });

  const saveItems = useMutation({
    mutationFn: async () => {
      setErr(null);
      if (!listingId) throw new Error('No draft yet');
      if (rows.length === 0) throw new Error('Add at least one item');
      const items = rows.map((r) => ({
        category_id: r.category_id,
        quantity: r.quantity,
        est_weight_g: r.est_weight_g,
        grade: r.grade || null,
      }));
      const { data, error } = await api.PATCH(`/v1/listings/${listingId}/items` as '/v1/listings/{id}/items', { body: { items } });
      if (error) throw error;
      return data;
    },
    onSuccess: () => {
      listing.refetch();
      suggestion.refetch();
    },
    onError: (e) => setErr(e instanceof Error ? e.message : 'Could not save items'),
  });

  const analyze = useMutation({
    mutationFn: async () => {
      setErr(null);
      const { data, error } = await api.POST(`/v1/listings/${listingId}:analyze` as '/v1/listings/{id}:publish');
      if (error) throw error;
      return data;
    },
    onSuccess: () => listing.refetch(),
    onError: (e) => setErr(e instanceof Error ? e.message : 'Could not start analysis'),
  });

  const publish = useMutation({
    mutationFn: async () => {
      setErr(null);
      const { data, error } = await api.POST(`/v1/listings/${listingId}:publish` as '/v1/listings/{id}:publish');
      if (error) throw error;
      return data;
    },
    onSuccess: () => router.push(`/listing/?id=${listingId}`),
    onError: (e) => setErr(e instanceof Error ? e.message : 'Could not publish'),
  });

  if (!ready) return <Spinner />;
  if (!token)
    return (
      <Card title="Sign in to sell">
        <p className="kb-muted">You need an account to list your scrap.</p>
        <a href="/signin/"><Button>Sign in</Button></a>
      </Card>
    );

  const categories = taxonomy.data?.categories ?? [];
  const catName = (id: string) => categories.find((c) => c.id === id)?.name ?? id;
  const gradesFor = (id: string) => categories.find((c) => c.id === id)?.grades ?? [];
  const analysisStatus = listing.data?.version.analysis.status;
  const analyzing = analysisStatus === 'QUEUED' || analysisStatus === 'RUNNING';

  function addRow() {
    const first = categories[0]?.id ?? '';
    setRows((r) => [...r, { category_id: first, grade: '', quantity: 1, est_weight_g: 1000 }]);
  }
  function updateRow(i: number, patch: Partial<ItemRow>) {
    setRows((r) => r.map((row, idx) => (idx === i ? { ...row, ...patch } : row)));
  }
  function removeRow(i: number) {
    setRows((r) => r.filter((_, idx) => idx !== i));
  }

  return (
    <>
      <h1 className="kb-h1">Sell your scrap</h1>
      {err ? <ErrorNote>{err}</ErrorNote> : null}

      {/* Step 1 — pickup address */}
      <Card title="1. Pickup address">
        {addresses.isLoading ? <Spinner /> : null}
        {!addNew ? (
          <>
            {(addresses.data?.length ?? 0) === 0 ? <p className="kb-muted">No saved addresses yet.</p> : null}
            <ul className="kb-list">
              {addresses.data?.map((a) => (
                <li key={a.id}>
                  <label className="kb-row" style={{ cursor: 'pointer' }}>
                    <input type="radio" name="addr" checked={addressId === a.id} onChange={() => setAddressId(a.id)} disabled={!!listingId} />
                    <span>
                      <strong>{a.label}</strong> — {a.text}
                      <br />
                      <span className="kb-muted">{a.landmark ? `${a.landmark} · ` : ''}{a.city}</span>
                    </span>
                  </label>
                </li>
              ))}
            </ul>
            {!listingId ? <Button variant="ghost" onClick={() => setAddNew(true)}>+ Add a new address</Button> : null}
          </>
        ) : (
          <>
            <Field label="Label"><input className="kb-input" value={aLabel} onChange={(e) => setALabel(e.target.value)} placeholder="Home, Shop…" /></Field>
            <Field label="Full address"><input className="kb-input" value={aText} onChange={(e) => setAText(e.target.value)} placeholder="Flat / street / area" /></Field>
            <Field label="Landmark (optional)"><input className="kb-input" value={aLandmark} onChange={(e) => setALandmark(e.target.value)} /></Field>
            <Field label="City (optional)"><input className="kb-input" value={aCity} onChange={(e) => setACity(e.target.value)} /></Field>
            <Field label="Location pin (lat, lng)"><input className="kb-input" value={aLatLng} onChange={(e) => setALatLng(e.target.value)} inputMode="decimal" placeholder="19.0760, 72.8777" /></Field>
            <p className="kb-muted">A map picker would go here; for now type the coordinates.</p>
            <div className="kb-row">
              <Button onClick={() => addAddress.mutate()} disabled={addAddress.isPending}>{addAddress.isPending ? 'Saving…' : 'Save address'}</Button>
              <Button variant="ghost" onClick={() => setAddNew(false)}>Cancel</Button>
            </div>
          </>
        )}
      </Card>

      {/* Step 2 — describe & create draft */}
      {!listingId ? (
        <Card title="2. Describe your lot">
          <Field label="What are you selling? (optional)">
            <textarea className="kb-input" rows={3} value={description} onChange={(e) => setDescription(e.target.value)} placeholder="e.g. old laptop, 2 ceiling fans, a sack of newspaper" />
          </Field>
          <Button onClick={() => createDraft.mutate()} disabled={!addressId || createDraft.isPending}>{createDraft.isPending ? 'Creating…' : 'Start listing'}</Button>
        </Card>
      ) : (
        <>
          {/* Step 3 — photos */}
          <Card title="3. Add photos">
            <p className="kb-muted">Take or upload a wide shot of everything, then close-ups of valuable items. Clear photos get better AI estimates and higher offers. Faces are blurred and location data removed before any vendor sees them.</p>
            <div className="kb-row">
              <PhotoUpload listingId={listingId!} kind="WIDE" source="CAMERA" label="📷 Take photo" onUploaded={() => listing.refetch()} />
              <PhotoUpload listingId={listingId!} kind="WIDE" source="GALLERY" label="Upload from gallery" onUploaded={() => listing.refetch()} />
            </div>
            {listing.data && listing.data.images.length > 0 ? (
              <div className="kb-row" style={{ marginTop: 12 }}>
                {listing.data.images.map((im) =>
                  im.thumb_url || im.url ? <img key={im.id} src={im.thumb_url ?? im.url ?? ''} alt="" style={{ width: 72, height: 72, objectFit: 'cover', borderRadius: 8 }} /> : null,
                )}
              </div>
            ) : (
              <p className="kb-muted" style={{ marginTop: 8 }}>No photos yet.</p>
            )}
          </Card>

          {/* Step 4 — items */}
          <Card title="4. Add items">
            {taxonomy.isLoading ? <Spinner /> : null}
            {rows.length === 0 ? <p className="kb-muted">Add each kind of scrap with a rough weight. You can adjust at pickup.</p> : null}
            <ul className="kb-list">
              {rows.map((row, i) => (
                <li key={i}>
                  <div className="kb-grid-2">
                    <Field label="Category">
                      <select className="kb-input" value={row.category_id} onChange={(e) => updateRow(i, { category_id: e.target.value, grade: '' })}>
                        {categories.map((c) => (
                          <option key={c.id} value={c.id}>{c.name}{c.hazardous ? ' ⚠' : ''}</option>
                        ))}
                      </select>
                    </Field>
                    <Field label="Grade">
                      <select className="kb-input" value={row.grade} onChange={(e) => updateRow(i, { grade: e.target.value })} disabled={gradesFor(row.category_id).length === 0}>
                        <option value="">Any / unsure</option>
                        {gradesFor(row.category_id).map((g) => (
                          <option key={g.id} value={g.id}>{g.name}</option>
                        ))}
                      </select>
                    </Field>
                    <Field label="Quantity">
                      <input className="kb-input" type="number" min={1} value={row.quantity} onChange={(e) => updateRow(i, { quantity: Math.max(1, Number(e.target.value) || 1) })} />
                    </Field>
                    <Field label="Est. weight (grams)">
                      <input className="kb-input" type="number" min={0} value={row.est_weight_g} onChange={(e) => updateRow(i, { est_weight_g: Math.max(0, Number(e.target.value) || 0) })} />
                    </Field>
                  </div>
                  <Button variant="ghost" onClick={() => removeRow(i)}>Remove</Button>
                </li>
              ))}
            </ul>
            <div className="kb-row">
              <Button variant="ghost" onClick={addRow} disabled={categories.length === 0}>+ Add item</Button>
              <Button onClick={() => saveItems.mutate()} disabled={rows.length === 0 || saveItems.isPending}>{saveItems.isPending ? 'Saving…' : 'Save items'}</Button>
            </div>
          </Card>

          {/* Step 4 — AI itemization (optional) */}
          <Card title="5. AI check (optional)">
            <p className="kb-muted">Let KabadOS read your photos and description to itemize and verify. This can raise your verification level and the price.</p>
            <div className="kb-between">
              <Button variant="ghost" onClick={() => analyze.mutate()} disabled={analyze.isPending || analyzing}>
                {analyzing ? 'Analyzing…' : analyze.isPending ? 'Starting…' : 'Run AI analysis'}
              </Button>
              {analysisStatus ? <Badge tone={analysisStatus === 'DONE' ? 'good' : analysisStatus === 'FAILED_CLOSED' ? 'hazard' : 'warn'}>{analysisStatus.replace(/_/g, ' ').toLowerCase()}</Badge> : null}
            </div>
            {listing.data && listing.data.items.length > 0 ? (
              <ul className="kb-list">
                {listing.data.items.map((it) => (
                  <li key={it.id} className="kb-between">
                    <span>{catName(it.category_id)} · {it.quantity} · {formatWeight(it.est_weight_g ?? 0)}</span>
                    <span className="kb-row">
                      {it.hazard.is_hazardous ? <Badge tone="hazard">Hazardous</Badge> : null}
                      <LadderBadge level={it.ladder} />
                    </span>
                  </li>
                ))}
              </ul>
            ) : null}
          </Card>

          {/* Step 5 — price suggestion */}
          <Card title="6. Suggested price">
            {suggestion.isLoading ? <Spinner /> : null}
            {suggestion.data ? (
              suggestion.data.low != null && suggestion.data.high != null ? (
                <>
                  <p className="kb-price">{formatRange(suggestion.data.low, suggestion.data.high)}</p>
                  <p className="kb-muted">Based on {suggestion.data.vendor_count} nearby vendor{suggestion.data.vendor_count === 1 ? '' : 's'} · {suggestion.data.basis.replace(/_/g, ' ').toLowerCase()}</p>
                </>
              ) : (
                <p className="kb-muted">No price data yet — publish to get real offers.</p>
              )
            ) : null}
          </Card>

          {/* Step 6 — publish */}
          <Card title="7. Publish">
            <p className="kb-muted">Publishing shows your lot to verified vendors nearby. You will compare their offers before accepting any.</p>
            <Button onClick={() => publish.mutate()} disabled={publish.isPending || analyzing}>{publish.isPending ? 'Publishing…' : 'Publish listing'}</Button>
          </Card>
        </>
      )}
    </>
  );
}
