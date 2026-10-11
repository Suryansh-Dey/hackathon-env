'use client';
import { Badge, Button, Card, ErrorNote, LadderBadge, Spinner, formatDateTime, formatWeight } from '@kabados/ui';
import { useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { StaffGate, msg, opsGet, useMounted, useOpsApi, type AiReviewItem, type ListingView } from '../ops-api';

export default function AiReviewsPage() {
  return (
    <StaffGate>
      <AiReviewQueue />
    </StaffGate>
  );
}

function AiReviewQueue() {
  const api = useOpsApi();
  const mounted = useMounted();
  const [openId, setOpenId] = useState<string | null>(null);

  const q = useQuery({
    queryKey: ['ops-ai-reviews'],
    enabled: mounted,
    queryFn: () => opsGet<AiReviewItem[]>(api, '/v1/ops/ai-reviews'),
  });

  return (
    <>
      <h1 className="kb-h1">AI review queue</h1>
      <p className="kb-muted">Low-confidence assessments, failed-closed analyses and contested verdicts.</p>

      {q.isLoading ? <Spinner /> : null}
      {q.error ? <ErrorNote>{msg(q.error)}</ErrorNote> : null}
      {q.data && q.data.length === 0 ? <p className="kb-empty">Nothing flagged. Queue is clear.</p> : null}

      <ul className="kb-list">
        {q.data?.map((item) => {
          const active = openId === item.listing_id;
          return (
            <li key={`${item.listing_id}-${item.ver}`}>
              <div className="kb-between">
                <span>
                  <strong>{item.reason}</strong>
                  <br />
                  <span className="kb-muted">Listing {item.listing_id} · v{item.ver} · {formatDateTime(item.at)}</span>
                </span>
                <Button variant="ghost" onClick={() => setOpenId(active ? null : item.listing_id)}>
                  {active ? 'Close' : 'Open listing'}
                </Button>
              </div>
              {active ? <ListingPanel id={item.listing_id} /> : null}
            </li>
          );
        })}
      </ul>
    </>
  );
}

function ListingPanel({ id }: { id: string }) {
  const api = useOpsApi();
  const q = useQuery({
    queryKey: ['ops-listing', id],
    queryFn: () => opsGet<ListingView>(api, `/v1/ops/listings/${id}`),
  });

  if (q.isLoading) return <Spinner label="Loading listing…" />;
  if (q.error) return <ErrorNote>{msg(q.error)}</ErrorNote>;
  const v = q.data;
  if (!v) return null;

  return (
    <Card title={v.listing.title}>
      <div className="kb-row kb-between">
        <span className="kb-muted">{v.listing.categories.length} categories · {formatWeight(v.listing.total_est_weight_g)}</span>
        <span className="kb-row">
          <Badge tone={v.listing.contains_hazardous ? 'hazard' : 'neutral'}>{v.listing.contains_hazardous ? 'hazardous' : 'non-hazardous'}</Badge>
          <Badge>honesty {Math.round(v.seller_honesty * 100)}%</Badge>
        </span>
      </div>
      {v.version.description ? <p>{v.version.description}</p> : null}
      <p className="kb-muted">Analysis: {v.version.analysis.status.replaceAll('_', ' ').toLowerCase()} · proof rounds {v.version.analysis.proof_rounds}</p>

      <h3>Items</h3>
      <ul className="kb-list">
        {v.items.map((it) => (
          <li key={it.id} className="kb-between">
            <span>
              <strong>{it.category_id}</strong>{it.grade ? ` · ${it.grade}` : ''} × {it.quantity}
              {it.est_weight_g != null ? <span className="kb-muted"> · {formatWeight(it.est_weight_g)}</span> : null}
              {it.ai_confidence != null ? <span className="kb-muted"> · AI {Math.round(it.ai_confidence * 100)}%</span> : null}
            </span>
            <LadderBadge level={it.ladder} />
          </li>
        ))}
      </ul>

      <h3>Claims</h3>
      {v.claims.length === 0 ? <p className="kb-empty">No claims.</p> : (
        <ul className="kb-list">
          {v.claims.map((c) => (
            <li key={c.id}>
              <div className="kb-between">
                <span><strong>{c.text}</strong></span>
                <span className="kb-row">
                  {c.contested ? <Badge tone="warn">contested</Badge> : null}
                  <Badge tone={c.verdict === 'SUPPORTED' ? 'good' : c.verdict === 'INCONSISTENT' ? 'hazard' : 'warn'}>
                    {c.verdict.replaceAll('_', ' ').toLowerCase()}
                  </Badge>
                  <LadderBadge level={c.ladder_level} />
                </span>
              </div>
              <p className="kb-muted">{c.rationale} · confidence {Math.round(c.confidence * 100)}%</p>
            </li>
          ))}
        </ul>
      )}

      {v.images.some((im) => im.thumb_url || im.url) ? (
        <>
          <h3>Photos</h3>
          <div className="kb-row" style={{ flexWrap: 'wrap', gap: 8 }}>
            {v.images.map((im) => {
              const src = im.thumb_url ?? im.url;
              return src ? (
                // eslint-disable-next-line @next/next/no-img-element
                <img key={im.id} src={src} alt={im.kind} width={120} height={120} style={{ objectFit: 'cover', borderRadius: 8 }} />
              ) : null;
            })}
          </div>
        </>
      ) : null}
    </Card>
  );
}
