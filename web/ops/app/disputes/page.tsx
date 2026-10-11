'use client';
import { Badge, Button, Card, ErrorNote, Field, Spinner, formatDateTime, formatPaise, formatWeight } from '@kabados/ui';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import { StaffGate, msg, opsGet, opsPost, useMounted, useOpsApi, type DealEvidence, type Dispute } from '../ops-api';

const STATUSES = ['OPEN', 'IN_REVIEW', 'RESOLVED'] as const;
const DEAL_STATUSES = [
  'CREATED', 'PICKUP_SCHEDULED', 'JOB_ASSIGNED', 'AT_PICKUP', 'WEIGHED', 'LINE_REGRADED', 'SELLER_CONFIRMED',
  'AWAITING_PAYMENT', 'PAID', 'PAID_DIRECT', 'HANDED_OVER', 'IN_TRANSIT', 'DELIVERED', 'SETTLED', 'CLOSED',
  'SELLER_NO_SHOW', 'PICKUP_NO_SHOW', 'DISPUTED', 'CANCELLED',
] as const;

export default function DisputesPage() {
  return (
    <StaffGate>
      <DisputeQueue />
    </StaffGate>
  );
}

function DisputeQueue() {
  const api = useOpsApi();
  const mounted = useMounted();
  const [status, setStatus] = useState<string>('OPEN');
  const [open, setOpen] = useState<{ dealId: string; disputeId: string } | null>(null);

  const q = useQuery({
    queryKey: ['ops-disputes', status],
    enabled: mounted,
    queryFn: () => opsGet<Dispute[]>(api, '/v1/ops/disputes', { status }),
  });

  return (
    <>
      <div className="kb-between">
        <h1 className="kb-h1">Disputes</h1>
        <Field label="Status">
          <select className="kb-input" value={status} onChange={(e) => { setStatus(e.target.value); setOpen(null); }}>
            {STATUSES.map((s) => <option key={s} value={s}>{s.replaceAll('_', ' ').toLowerCase()}</option>)}
          </select>
        </Field>
      </div>

      {q.isLoading ? <Spinner /> : null}
      {q.error ? <ErrorNote>{msg(q.error)}</ErrorNote> : null}
      {q.data && q.data.length === 0 ? <p className="kb-empty">No disputes with this status.</p> : null}

      <ul className="kb-list">
        {q.data?.map((d) => {
          const active = open?.disputeId === d.id;
          return (
            <li key={d.id}>
              <div className="kb-between">
                <span>
                  <strong>{d.kind.replaceAll('_', ' ')}</strong>
                  <br />
                  <span className="kb-muted">{d.description}</span>
                  <br />
                  <span className="kb-muted">Deal {d.deal_id} · opened by {d.opened_by} · {formatDateTime(d.created_at)}</span>
                </span>
                <span className="kb-row">
                  <Badge tone={d.status === 'RESOLVED' ? 'good' : d.status === 'OPEN' ? 'hazard' : 'warn'}>
                    {d.status.replaceAll('_', ' ').toLowerCase()}
                  </Badge>
                  <Button variant="ghost" onClick={() => setOpen(active ? null : { dealId: d.deal_id, disputeId: d.id })}>
                    {active ? 'Close' : 'Open evidence'}
                  </Button>
                </span>
              </div>
              {active ? <DealPanel dealId={d.deal_id} dispute={d} /> : null}
            </li>
          );
        })}
      </ul>
    </>
  );
}

function DealPanel({ dealId, dispute }: { dealId: string; dispute: Dispute }) {
  const api = useOpsApi();
  const qc = useQueryClient();
  const [resolution, setResolution] = useState('');
  const [resume, setResume] = useState<string>(dispute.resume_status);

  const q = useQuery({
    queryKey: ['ops-deal', dealId],
    queryFn: () => opsGet<DealEvidence>(api, `/v1/ops/deals/${dealId}`),
  });

  const resolve = useMutation({
    mutationFn: () =>
      opsPost(api, `/v1/ops/disputes/${dealId}/${dispute.id}:resolve`, {
        resolution,
        resume: resume || undefined,
      }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['ops-disputes'] });
      qc.invalidateQueries({ queryKey: ['ops-deal', dealId] });
    },
  });

  if (q.isLoading) return <Spinner label="Loading evidence…" />;
  if (q.error) return <ErrorNote>{msg(q.error)}</ErrorNote>;
  const e = q.data;
  if (!e) return null;

  return (
    <Card title={`Deal ${dealId}`}>
      <div className="kb-row kb-between">
        <span>Status <Badge>{e.deal.status.replaceAll('_', ' ').toLowerCase()}</Badge></span>
        <span className="kb-price">{formatPaise(e.deal.agreed_total_low)} – {formatPaise(e.deal.agreed_total_high)}</span>
      </div>
      {e.deal.final_amount != null ? <p className="kb-muted">Final amount {formatPaise(e.deal.final_amount)}</p> : null}

      <h3>Lines</h3>
      {e.lines.length === 0 ? <p className="kb-empty">No lines.</p> : (
        <table className="kb-list" style={{ width: '100%' }}>
          <thead><tr><th>#</th><th>Category</th><th>Grade</th><th>Qty</th><th>Weighed</th><th>Amount</th><th>Status</th></tr></thead>
          <tbody>
            {e.lines.map((l) => (
              <tr key={l.n}>
                <td>{l.n}</td>
                <td>{l.category_id}</td>
                <td>{l.grade ?? '—'}</td>
                <td>{l.quantity}</td>
                <td>{l.weighed_g != null ? formatWeight(l.weighed_g) : '—'}</td>
                <td>{l.amount != null ? formatPaise(l.amount) : '—'}</td>
                <td>{l.status.toLowerCase()}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      <h3>Weighings</h3>
      {e.weighings.length === 0 ? <p className="kb-empty">None.</p> : (
        <ul className="kb-list">
          {e.weighings.map((w, i) => (
            <li key={i} className="kb-between">
              <span>Line {w.line_n} · {formatWeight(w.reading_g)}{w.ai_reading_g != null ? ` (AI ${formatWeight(w.ai_reading_g)})` : ''}</span>
              <span className="kb-muted">{formatDateTime(w.at)} · {w.by_user}</span>
            </li>
          ))}
        </ul>
      )}

      <h3>Job events</h3>
      {e.job_events.length === 0 ? <p className="kb-empty">None.</p> : (
        <ul className="kb-list">
          {e.job_events.map((ev, i) => (
            <li key={i} className="kb-between">
              <span><strong>{ev.kind.replaceAll('_', ' ')}</strong>{ev.note ? ` — ${ev.note}` : ''}{ev.mock_location ? ' ⚠ mock GPS' : ''}</span>
              <span className="kb-muted">{formatDateTime(ev.at)}</span>
            </li>
          ))}
        </ul>
      )}

      <h3>Audit trail</h3>
      {e.audit.length === 0 ? <p className="kb-empty">None.</p> : (
        <ul className="kb-list">
          {e.audit.map((a, i) => (
            <li key={i} className="kb-between">
              <span><strong>{a.action}</strong> by {a.actor} <span className="kb-muted">on {a.subject}</span></span>
              <span className="kb-muted">{formatDateTime(a.at)}</span>
            </li>
          ))}
        </ul>
      )}

      {e.ratings.length > 0 ? (
        <>
          <h3>Ratings</h3>
          <ul className="kb-list">
            {e.ratings.map((r, i) => (
              <li key={i} className="kb-between">
                <span>{r.stars}★ {r.comment ? `— ${r.comment}` : ''}</span>
                <span className="kb-muted">{r.from_user} → {r.to_user}</span>
              </li>
            ))}
          </ul>
        </>
      ) : null}

      {dispute.status !== 'RESOLVED' ? (
        <Card title="Resolve dispute">
          <Field label="Resolution">
            <input className="kb-input" value={resolution} onChange={(ev) => setResolution(ev.target.value)} placeholder="How this was resolved" />
          </Field>
          <Field label="Resume deal at">
            <select className="kb-input" value={resume} onChange={(ev) => setResume(ev.target.value)}>
              <option value="">Don’t resume</option>
              {DEAL_STATUSES.map((s) => <option key={s} value={s}>{s.replaceAll('_', ' ').toLowerCase()}</option>)}
            </select>
          </Field>
          {resolve.error ? <ErrorNote>{msg(resolve.error)}</ErrorNote> : null}
          <Button onClick={() => resolve.mutate()} disabled={resolve.isPending || !resolution}>
            {resolve.isPending ? 'Resolving…' : 'Resolve'}
          </Button>
        </Card>
      ) : (
        <p className="kb-muted">Resolution: {dispute.resolution ?? '—'}</p>
      )}
    </Card>
  );
}
