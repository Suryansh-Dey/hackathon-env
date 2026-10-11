'use client';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Badge, Button, Card, ErrorNote, Field, Spinner, formatDate, stars, useApi, useAuth } from '@kabados/ui';
import { useEffect, useState } from 'react';
import { msg } from './lib/api';

/** Hazardous-waste authorization (state PCB) — unlocks hazardous categories. */
function HazardousAuthCard({ authorized, validUntil }: { authorized: boolean; validUntil?: string | null }) {
  const api = useApi();
  const qc = useQueryClient();
  const [no, setNo] = useState('');
  const [state, setState] = useState('');
  const [until, setUntil] = useState('');
  const [tieUp, setTieUp] = useState(false);
  const save = useMutation({
    mutationFn: async () => {
      if (!no.trim() || !state.trim() || !until) throw new Error('Fill the authorization number, state and valid-until date.');
      const { error } = await api.POST('/v1/vendor/authorizations', {
        body: {
          authorization_no: no.trim(),
          state: state.trim(),
          // End of the chosen day, so "valid until today" still counts today.
          valid_until: new Date(`${until}T23:59:59`).toISOString(),
          via_tie_up: tieUp,
        },
      });
      if (error) throw error;
    },
    onSuccess: () => {
      setNo('');
      qc.invalidateQueries({ queryKey: ['me'] });
    },
  });
  const expired = !authorized && !!validUntil;
  return (
    <Card title="Hazardous-waste authorization">
      <p>
        <Badge tone={authorized ? 'good' : expired ? 'hazard' : 'warn'}>
          {authorized ? 'Authorised' : expired ? 'Expired' : 'Not authorised'}
        </Badge>{' '}
        {validUntil ? <span className="kb-muted">valid until {formatDate(validUntil)}</span> : null}
      </p>
      <p className="kb-muted">
        Hazardous categories (batteries, CFL tubes, CRTs…) need a valid State Pollution Control Board authorization{authorized ? '. Add a new one to renew.' : ' before you can quote or buy them.'}
      </p>
      <div className="kb-grid-2">
        <Field label="Authorization number">
          <input className="kb-input" value={no} onChange={(e) => setNo(e.target.value)} placeholder="e.g. HWM/MH/2026/0142" />
        </Field>
        <Field label="State">
          <input className="kb-input" value={state} onChange={(e) => setState(e.target.value)} placeholder="e.g. MH" />
        </Field>
      </div>
      <Field label="Valid until">
        <input className="kb-input" type="date" value={until} onChange={(e) => setUntil(e.target.value)} style={{ maxWidth: 200 }} />
      </Field>
      <label style={{ display: 'flex', gap: 8, alignItems: 'center', margin: '8px 0', cursor: 'pointer' }}>
        <input type="checkbox" checked={tieUp} onChange={(e) => setTieUp(e.target.checked)} /> Held via a tie-up with an authorised recycler
      </label>
      {save.error ? <ErrorNote>{msg(save.error)}</ErrorNote> : null}
      {save.isSuccess ? <p className="kb-muted">Authorization saved.</p> : null}
      <Button disabled={save.isPending} onClick={() => save.mutate()}>{save.isPending ? 'Saving…' : 'Save authorization'}</Button>
    </Card>
  );
}

export default function Home() {
  const { token } = useAuth();
  const api = useApi();
  const [ready, setReady] = useState(false);
  useEffect(() => setReady(true), []);
  const enabled = ready && !!token;

  const me = useQuery({
    queryKey: ['me'],
    enabled,
    queryFn: async () => {
      const { data, error } = await api.GET('/v1/me');
      if (error) throw error;
      return data;
    },
  });
  const matches = useQuery({
    queryKey: ['matches'],
    enabled,
    queryFn: async () => {
      const { data, error } = await api.GET('/v1/vendor/matches');
      if (error) throw error;
      return data;
    },
  });
  const offers = useQuery({
    queryKey: ['vendor-offers-all'],
    enabled,
    queryFn: async () => {
      const { data, error } = await api.GET('/v1/vendor/offers', { params: {} as never });
      if (error) throw error;
      return data;
    },
  });
  const deals = useQuery({
    queryKey: ['deals'],
    enabled,
    queryFn: async () => {
      const { data, error } = await api.GET('/v1/deals');
      if (error) throw error;
      return data;
    },
  });

  if (!ready) return <Spinner />;
  if (!token)
    return (
      <Card title="KabadOS for vendors">
        <p className="kb-muted">Browse verified scrap listings near your yard, publish your rate card, send offers, and run pickups end to end.</p>
        <p><a href="/signin/"><Button>Sign in</Button></a></p>
      </Card>
    );

  const v = me.data?.vendor;
  const activeOffers = offers.data?.filter((o) => o.status === 'ACTIVE').length ?? 0;
  const openDeals = deals.data?.filter((d) => !['CLOSED', 'SETTLED', 'CANCELLED'].includes(d.status)).length ?? 0;

  return (
    <>
      <div className="kb-between">
        <h1 className="kb-h1">{v?.business_name ?? 'Vendor dashboard'}</h1>
        <a href="/feed/"><Button>Browse listings</Button></a>
      </div>

      {me.isLoading ? <Spinner /> : null}
      {me.error ? <ErrorNote>{msg(me.error)}</ErrorNote> : null}

      {v ? (
        <Card>
          <div className="kb-between">
            <span><strong>{v.city}</strong> · {stars(v.rating)} <span className="kb-muted">({v.rating.toFixed(1)})</span></span>
            <Badge tone={v.active_rule_set_version ? 'good' : 'warn'}>
              {v.active_rule_set_version ? `Rate card v${v.active_rule_set_version} live` : 'No rate card published'}
            </Badge>
          </div>
          <p className="kb-muted">
            Reliability {Math.round(v.reliability * 100)}% ·{' '}
            {v.hazardous_authorized ? 'Authorised for hazardous categories' : 'Not authorised for hazardous'}
          </p>
          {!v.active_rule_set_version ? (
            <p><a href="/rate-card/"><Button>Publish your rate card</Button></a> <span className="kb-muted">Listings can’t be quoted until a rate card is live.</span></p>
          ) : null}
        </Card>
      ) : (
        <Card title="Finish setup">
          <p className="kb-muted">Your account isn’t a vendor yet.</p>
          <a href="/signin/"><Button>Set up your yard</Button></a>
        </Card>
      )}

      <div className="kb-grid-2">
        <a href="/matches/" style={{ textDecoration: 'none', color: 'inherit' }}>
          <Card title="First-look leads">
            <p className="kb-price">{matches.data?.length ?? '–'}</p>
            <p className="kb-muted">New matched listings</p>
          </Card>
        </a>
        <a href="/offers/" style={{ textDecoration: 'none', color: 'inherit' }}>
          <Card title="Active offers">
            <p className="kb-price">{activeOffers}</p>
            <p className="kb-muted">Awaiting a seller decision</p>
          </Card>
        </a>
        <a href="/deals/" style={{ textDecoration: 'none', color: 'inherit' }}>
          <Card title="Open deals">
            <p className="kb-price">{openDeals}</p>
            <p className="kb-muted">Pickups in progress</p>
          </Card>
        </a>
        <a href="/rate-card/" style={{ textDecoration: 'none', color: 'inherit' }}>
          <Card title="Rate card">
            <p className="kb-price">{v?.active_rule_set_version ? `v${v.active_rule_set_version}` : '—'}</p>
            <p className="kb-muted">Edit prices &amp; coverage</p>
          </Card>
        </a>
      </div>

      {v ? <HazardousAuthCard authorized={v.hazardous_authorized} validUntil={v.hazardous_valid_until} /> : null}
    </>
  );
}
