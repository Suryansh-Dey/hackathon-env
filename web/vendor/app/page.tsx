'use client';
import { useQuery } from '@tanstack/react-query';
import { Badge, Button, Card, Spinner, stars, useApi, useAuth } from '@kabados/ui';
import { useEffect, useState } from 'react';

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
    </>
  );
}
