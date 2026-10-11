'use client';
import { Badge, Button, Card, ErrorNote, Field, LadderBadge, PhotoUpload, Spinner, formatRange, formatWeight, problemMessage, useApi, useAuth } from '@kabados/ui';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useRouter } from 'next/navigation';
import { useEffect, useState } from 'react';
import type { paths } from '@kabados/api-client';
import { ProfileGate } from '../profile';

type ListingView = paths['/v1/listings/{id}']['get']['responses'][200]['content']['application/json'];
type Item = ListingView['items'][number];
type ItemInput = paths['/v1/listings/{id}/items']['patch']['requestBody']['content']['application/json']['items'][number];

// One editable item row. `id` keeps an existing (e.g. AI-found) item so the
// server preserves its AI fields, hazard flags and verification level.
type ItemRow = {
  id: string | null;
  category_id: string;
  grade: string;
  quantity: number;
  weight_kg: string;
  attributes?: ItemInput['attributes'];
  note?: string | null;
};

const PUBLISHABLE = ['DRAFT', 'READY', 'NEEDS_INPUT', 'EXPIRED', 'CANCELLED'];

function rowFromItem(it: Item): ItemRow {
  return {
    id: it.id,
    category_id: it.category_id,
    grade: it.grade ?? '',
    quantity: it.quantity,
    weight_kg: it.est_weight_g != null ? String(it.est_weight_g / 1000) : '',
    attributes: it.attributes,
    note: it.note ?? null,
  };
}

/** "1.5" (kg) → 1500 g; null when empty or not a positive number. */
function gramsOf(kg: string): number | null {
  const n = Number(kg);
  if (!kg.trim() || !Number.isFinite(n) || n <= 0) return null;
  return Math.max(1, Math.round(n * 1000));
}

export default function NewListing() {
  const { token } = useAuth();
  const api = useApi();
  const qc = useQueryClient();
  const router = useRouter();
  const [ready, setReady] = useState(false);

  // Draft the listing is attached to once created (step 2 onward). Kept in the
  // URL (?id=) so a reload resumes the same draft.
  const [listingId, setListingId] = useState<string | null>(null);
  useEffect(() => {
    setListingId(new URLSearchParams(window.location.search).get('id'));
    setReady(true);
  }, []);

  // Step 1: address
  const [addressId, setAddressId] = useState<string>('');
  const [addNew, setAddNew] = useState(false);
  const [aLabel, setALabel] = useState('Home');
  const [aText, setAText] = useState('');
  const [aLandmark, setALandmark] = useState('');
  const [aCity, setACity] = useState('');
  const [aLatLng, setALatLng] = useState('');
  const [locating, setLocating] = useState(false);

  // Step 2: description
  const [description, setDescription] = useState('');

  // Step 4: items (seeded from the listing's items, including AI-found ones)
  const [rows, setRows] = useState<ItemRow[]>([]);
  const [dirty, setDirty] = useState(false);

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
    enabled: ready && !!token && !!listingId,
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
    enabled: ready && !!token && !!listingId,
    queryFn: async () => {
      const { data, error } = await api.GET(`/v1/listings/${listingId}/price-suggestion` as '/v1/listings/{id}/price-suggestion');
      if (error) throw error;
      return data;
    },
  });

  // Mirror the server's items into the editor until the seller starts editing.
  useEffect(() => {
    if (listing.data && !dirty) setRows(listing.data.items.map(rowFromItem));
  }, [listing.data, dirty]);

  const categories = taxonomy.data?.categories ?? [];
  const catName = (id: string) => categories.find((c) => c.id === id)?.name ?? id;
  const gradesFor = (id: string) => categories.find((c) => c.id === id)?.grades ?? [];

  function locateMe() {
    setErr(null);
    if (typeof navigator === 'undefined' || !navigator.geolocation) {
      setErr('Location is not available in this browser. Type the coordinates instead.');
      return;
    }
    setLocating(true);
    navigator.geolocation.getCurrentPosition(
      (pos) => {
        setALatLng(`${pos.coords.latitude.toFixed(6)}, ${pos.coords.longitude.toFixed(6)}`);
        setLocating(false);
      },
      (e) => {
        setErr(e.code === e.PERMISSION_DENIED ? 'Location permission was denied. Type the coordinates instead.' : 'Could not get your location. Type the coordinates instead.');
        setLocating(false);
      },
      { enableHighAccuracy: true, timeout: 15_000, maximumAge: 60_000 },
    );
  }

  const addAddress = useMutation({
    mutationFn: async () => {
      setErr(null);
      const parts = aLatLng.split(',').map((s) => Number(s.trim()));
      if (parts.length !== 2 || parts.some((n) => Number.isNaN(n))) throw new Error('Tap “Use my current location”, or enter the pin as "lat, lng", e.g. 19.07, 72.87');
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
    onError: (e) => setErr(problemMessage(e, 'Could not save the address')),
  });

  const createDraft = useMutation({
    mutationFn: async () => {
      setErr(null);
      if (!addressId) throw new Error('Pick a pickup address first');
      const { data, error } = await api.POST('/v1/listings', { body: { address_id: addressId, description: description || null } });
      if (error) throw error;
      return data;
    },
    onSuccess: (data) => {
      qc.setQueryData(['listing', data.listing.id], data);
      qc.invalidateQueries({ queryKey: ['my-listings'] });
      setListingId(data.listing.id);
      window.history.replaceState(null, '', `/new/?id=${data.listing.id}`);
    },
    onError: (e) => setErr(problemMessage(e, 'Could not create the listing')),
  });

  async function putItems(): Promise<ListingView> {
    if (!listingId) throw new Error('No draft yet');
    const items: ItemInput[] = rows.map((r) => ({
      id: r.id,
      category_id: r.category_id,
      grade: r.grade || null,
      quantity: r.quantity,
      est_weight_g: gramsOf(r.weight_kg),
      attributes: r.attributes ?? {},
      note: r.note ?? null,
    }));
    const { data, error } = await api.PATCH(`/v1/listings/${listingId}/items` as '/v1/listings/{id}/items', { body: { items } });
    if (error) throw error;
    return data;
  }

  function itemsSaved(data: ListingView) {
    qc.setQueryData(['listing', listingId], data);
    setDirty(false);
    suggestion.refetch();
  }

  const saveItems = useMutation({
    mutationFn: async () => {
      setErr(null);
      if (rows.length === 0) throw new Error('Add at least one item');
      return putItems();
    },
    onSuccess: itemsSaved,
    onError: (e) => setErr(problemMessage(e, 'Could not save items')),
  });

  const analyze = useMutation({
    mutationFn: async () => {
      setErr(null);
      // Keep any unsaved edits: the AI works from the saved items and photos.
      if (dirty && rows.length > 0) itemsSaved(await putItems());
      const { data, error } = await api.POST(`/v1/listings/${listingId}:analyze` as '/v1/listings/{id}:publish');
      if (error) throw error;
      return data;
    },
    onSuccess: (data) => qc.setQueryData(['listing', listingId], data),
    onError: (e) => setErr(problemMessage(e, 'Could not start analysis')),
  });

  const publish = useMutation({
    mutationFn: async () => {
      setErr(null);
      if (rows.length === 0) throw new Error('Add at least one item before publishing');
      const missing = rows.find((r) => gramsOf(r.weight_kg) == null);
      if (missing) throw new Error(`Add an estimated weight for ${catName(missing.category_id)} before publishing`);
      if (dirty) itemsSaved(await putItems());
      const { data, error } = await api.POST(`/v1/listings/${listingId}:publish` as '/v1/listings/{id}:publish');
      if (error) throw error;
      return data;
    },
    onSuccess: (data) => {
      qc.setQueryData(['listing', listingId], data);
      qc.invalidateQueries({ queryKey: ['my-listings'] });
      router.push(`/listing/?id=${listingId}`);
    },
    onError: (e) => setErr(problemMessage(e, 'Could not publish')),
  });

  if (!ready) return <Spinner />;
  if (!token)
    return (
      <Card title="Sign in to sell">
        <p className="kb-muted">You need an account to list your scrap.</p>
        <a href="/signin/"><Button>Sign in</Button></a>
      </Card>
    );

  const l = listing.data;
  const analysisStatus = l?.version.analysis.status;
  const analyzing = analysisStatus === 'QUEUED' || analysisStatus === 'RUNNING' || l?.listing.status === 'ANALYZING';
  const status = l?.listing.status;
  const canPublish = !!status && PUBLISHABLE.includes(status);
  const itemById = new Map((l?.items ?? []).map((it) => [it.id, it]));
  const weightNotes = new Set(l?.weight_notes ?? []);
  const missingWeight = rows.some((r) => gramsOf(r.weight_kg) == null);

  function addRow() {
    const first = categories[0]?.id ?? '';
    setDirty(true);
    setRows((r) => [...r, { id: null, category_id: first, grade: '', quantity: 1, weight_kg: '' }]);
  }
  function updateRow(i: number, patch: Partial<ItemRow>) {
    setDirty(true);
    setRows((r) => r.map((row, idx) => (idx === i ? { ...row, ...patch } : row)));
  }
  function removeRow(i: number) {
    setDirty(true);
    setRows((r) => r.filter((_, idx) => idx !== i));
  }

  return (
    <ProfileGate>
      <h1 className="kb-h1">Sell your scrap</h1>
      {err ? <ErrorNote>{err}</ErrorNote> : null}
      {listingId && listing.error ? <ErrorNote>{problemMessage(listing.error, 'Could not load this draft')}</ErrorNote> : null}

      {/* Step 1 — pickup address */}
      <Card title="1. Pickup address">
        {listingId ? (
          <p className="kb-muted">Pickup address is set for this listing. <a href="/new/">Start a new listing</a> to use a different one.</p>
        ) : (
          <>
            {addresses.isLoading ? <Spinner /> : null}
            {addresses.error ? <ErrorNote>{problemMessage(addresses.error, 'Could not load your addresses')}</ErrorNote> : null}
            {!addNew ? (
              <>
                {(addresses.data?.length ?? 0) === 0 ? <p className="kb-muted">No saved addresses yet.</p> : null}
                <ul className="kb-list">
                  {addresses.data?.map((a) => (
                    <li key={a.id}>
                      <label className="kb-row" style={{ cursor: 'pointer' }}>
                        <input type="radio" name="addr" checked={addressId === a.id} onChange={() => setAddressId(a.id)} />
                        <span>
                          <strong>{a.label}</strong> — {a.text}
                          <br />
                          <span className="kb-muted">{a.landmark ? `${a.landmark} · ` : ''}{a.city}</span>
                        </span>
                      </label>
                    </li>
                  ))}
                </ul>
                <Button variant="ghost" onClick={() => setAddNew(true)}>+ Add a new address</Button>
              </>
            ) : (
              <>
                <Field label="Label"><input className="kb-input" value={aLabel} onChange={(e) => setALabel(e.target.value)} placeholder="Home, Shop…" /></Field>
                <Field label="Full address"><input className="kb-input" value={aText} onChange={(e) => setAText(e.target.value)} placeholder="Flat / street / area" /></Field>
                <Field label="Landmark (optional)"><input className="kb-input" value={aLandmark} onChange={(e) => setALandmark(e.target.value)} /></Field>
                <Field label="City (optional)"><input className="kb-input" value={aCity} onChange={(e) => setACity(e.target.value)} /></Field>
                <div style={{ marginBottom: 10 }}>
                  <Button variant="ghost" onClick={locateMe} disabled={locating}>{locating ? 'Finding you…' : '📍 Use my current location'}</Button>
                </div>
                <Field label="Location pin (lat, lng)"><input className="kb-input" value={aLatLng} onChange={(e) => setALatLng(e.target.value)} inputMode="decimal" placeholder="19.0760, 72.8777" /></Field>
                <p className="kb-muted">Stand where the pickup happens and tap “Use my current location”, or type the coordinates (e.g. copied from a maps app). Only nearby vendors see an approximate area until a deal is made.</p>
                <div className="kb-row">
                  <Button onClick={() => addAddress.mutate()} disabled={addAddress.isPending}>{addAddress.isPending ? 'Saving…' : 'Save address'}</Button>
                  <Button variant="ghost" onClick={() => setAddNew(false)}>Cancel</Button>
                </div>
              </>
            )}
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
      ) : listing.isLoading ? (
        <Spinner />
      ) : (
        <>
          {/* Step 3 — photos */}
          <Card title="2. Add photos">
            <p className="kb-muted">Take or upload a wide shot of everything, then close-ups of valuable items. Clear photos get better AI estimates and higher offers. Faces are blurred and location data removed before any vendor sees them.</p>
            <div className="kb-row">
              <PhotoUpload listingId={listingId} kind="WIDE" source="CAMERA" label="📷 Take photo" onUploaded={() => listing.refetch()} />
              <PhotoUpload listingId={listingId} kind="WIDE" source="GALLERY" label="Upload from gallery" onUploaded={() => listing.refetch()} />
            </div>
            {l && l.images.length > 0 ? (
              <div className="kb-row" style={{ marginTop: 12 }}>
                {l.images.map((im) =>
                  im.thumb_url || im.url ? <img key={im.id} src={im.thumb_url ?? im.url ?? ''} alt="" style={{ width: 72, height: 72, objectFit: 'cover', borderRadius: 8 }} /> : null,
                )}
              </div>
            ) : (
              <p className="kb-muted" style={{ marginTop: 8 }}>No photos yet.</p>
            )}
          </Card>

          {/* Step 3 — AI itemization (optional) */}
          <Card title="3. AI check (optional)">
            <p className="kb-muted">Let KabadOS read your photos and description to find and verify your items. It fills the item list below for you to check, and can raise your verification level and the price.</p>
            <div className="kb-between">
              <Button variant="ghost" onClick={() => analyze.mutate()} disabled={analyze.isPending || analyzing}>
                {analyzing ? 'Analyzing…' : analyze.isPending ? 'Starting…' : 'Run AI analysis'}
              </Button>
              {analysisStatus ? <Badge tone={analysisStatus === 'DONE' ? 'good' : analysisStatus === 'FAILED_CLOSED' ? 'hazard' : 'warn'}>{analysisStatus.replace(/_/g, ' ').toLowerCase()}</Badge> : null}
            </div>
            {l && l.proof_requests.some((p) => p.status === 'OPEN') ? (
              <p className="kb-muted">The AI asked for extra proof photos to raise your price — you can add them on the <a href={`/listing/?id=${listingId}`}>listing page</a> after publishing, or skip them.</p>
            ) : null}
          </Card>

          {/* Step 4 — items */}
          <Card title="4. Check your items">
            {taxonomy.isLoading ? <Spinner /> : null}
            {rows.length === 0 ? <p className="kb-muted">Add each kind of scrap with a rough weight, or run the AI check above. You can adjust at pickup.</p> : null}
            {dirty && l && l.items.length > 0 ? (
              <p className="kb-muted">
                You have unsaved changes.{' '}
                <a href="#" onClick={(e) => { e.preventDefault(); setDirty(false); }}>Discard them and reload the saved items</a>
              </p>
            ) : null}
            <ul className="kb-list">
              {rows.map((row, i) => {
                const it = row.id ? itemById.get(row.id) : undefined;
                const pw = it?.plausible_weight;
                const perPiece = it?.unit === 'PIECE';
                const sameCat = it && it.category_id === row.category_id;
                const typicalG = pw ? Math.round(((pw.range.min + pw.range.max) / 2) * (perPiece ? row.quantity : 1)) : null;
                return (
                  <li key={row.id ?? `new-${i}`}>
                    {it ? (
                      <div className="kb-row" style={{ marginBottom: 8 }}>
                        {it.source !== 'SELLER' ? <Badge tone="good">AI found{it.ai_confidence != null ? ` · ${Math.round(it.ai_confidence * 100)}%` : ''}</Badge> : null}
                        {sameCat && it.hazard.is_hazardous ? <Badge tone="hazard">Hazardous{it.hazard.class ? ` · ${it.hazard.class.replace(/_/g, ' ').toLowerCase()}` : ''}</Badge> : null}
                        {sameCat ? <LadderBadge level={it.ladder} /> : null}
                      </div>
                    ) : null}
                    {sameCat && it.hazard.handling ? <p className="kb-muted">⚠ {it.hazard.handling}</p> : null}
                    {row.note ? <p className="kb-muted">Note: {row.note}</p> : null}
                    <div className="kb-grid kb-grid-2">
                      <Field label="Category">
                        <select className="kb-input" value={row.category_id} onChange={(e) => updateRow(i, { category_id: e.target.value, grade: '' })}>
                          {categories.some((c) => c.id === row.category_id) ? null : <option value={row.category_id}>{row.category_id}</option>}
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
                        <input className="kb-input" type="number" min={1} value={row.quantity} onChange={(e) => updateRow(i, { quantity: Math.max(1, Math.floor(Number(e.target.value)) || 1) })} />
                      </Field>
                      <Field label="Est. weight of this line (kg) — required">
                        <input className="kb-input" type="number" min={0} step="0.1" inputMode="decimal" value={row.weight_kg} onChange={(e) => updateRow(i, { weight_kg: e.target.value })} placeholder="e.g. 2.5" />
                      </Field>
                    </div>
                    {sameCat && pw ? (
                      <p className="kb-muted">
                        Typical weight: {formatWeight(pw.range.min)} – {formatWeight(pw.range.max)}{perPiece ? ' per piece' : ''}
                        {pw.source === 'MODEL_ESTIMATE' ? ' (AI estimate)' : ''}
                        {typicalG ? (
                          <>
                            {' · '}
                            <a href="#" onClick={(e) => { e.preventDefault(); updateRow(i, { weight_kg: String(typicalG / 1000) }); }}>use {formatWeight(typicalG)}</a>
                          </>
                        ) : null}
                      </p>
                    ) : null}
                    {sameCat && !dirty && weightNotes.has(it.id) ? <p className="kb-muted">⚠ Your weight is outside the usual range for this item. That's OK — it will be weighed at pickup.</p> : null}
                    {gramsOf(row.weight_kg) == null ? <p className="kb-muted">Add a weight to publish.</p> : null}
                    <Button variant="ghost" onClick={() => removeRow(i)}>Remove</Button>
                  </li>
                );
              })}
            </ul>
            <div className="kb-row">
              <Button variant="ghost" onClick={addRow} disabled={categories.length === 0 || analyzing}>+ Add item</Button>
              <Button onClick={() => saveItems.mutate()} disabled={rows.length === 0 || !dirty || saveItems.isPending || analyzing}>{saveItems.isPending ? 'Saving…' : dirty ? 'Save items' : 'Saved'}</Button>
            </div>
          </Card>

          {/* Step 5 — price suggestion */}
          <Card title="5. Suggested price">
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
            <p className="kb-muted">You can set your own asking price on the listing page.</p>
          </Card>

          {/* Step 6 — publish */}
          <Card title="6. Publish">
            {canPublish ? (
              <>
                <p className="kb-muted">Publishing shows your lot to verified vendors nearby. You will compare their offers before accepting any.</p>
                {missingWeight ? <p className="kb-muted">Every item needs an estimated weight before you can publish.</p> : null}
                <Button onClick={() => publish.mutate()} disabled={publish.isPending || analyzing || rows.length === 0 || missingWeight}>{publish.isPending ? 'Publishing…' : 'Publish listing'}</Button>
              </>
            ) : (
              <p className="kb-muted">
                {analyzing ? 'Wait for the AI check to finish. ' : status ? `This listing is ${status.replace(/_/g, ' ').toLowerCase()}. ` : ''}
                <a href={`/listing/?id=${listingId}`}>Open the listing</a>
              </p>
            )}
          </Card>
        </>
      )}
    </ProfileGate>
  );
}
