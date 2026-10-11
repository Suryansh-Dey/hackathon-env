'use client';
import { Badge, Button, Card, ErrorNote, Field, Spinner, formatDate, formatDateTime } from '@kabados/ui';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import { StaffGate, msg, opsGet, opsPost, useMounted, useOpsApi, type KycDocument, type KycItem } from '../ops-api';

const STATUSES = ['PENDING_MANUAL_REVIEW', 'SUBMITTED', 'APPROVED', 'REJECTED', 'NOT_STARTED'] as const;

export default function KycPage() {
  return (
    <StaffGate>
      <KycQueue />
    </StaffGate>
  );
}

function KycQueue() {
  const api = useOpsApi();
  const mounted = useMounted();
  const qc = useQueryClient();
  const [status, setStatus] = useState<string>('PENDING_MANUAL_REVIEW');

  const q = useQuery({
    queryKey: ['ops-kyc', status],
    enabled: mounted,
    queryFn: () => opsGet<KycItem[]>(api, '/v1/ops/kyc', { status }),
  });

  return (
    <>
      <div className="kb-between">
        <h1 className="kb-h1">KYC review</h1>
        <Field label="Status">
          <select className="kb-input" value={status} onChange={(e) => setStatus(e.target.value)}>
            {STATUSES.map((s) => (
              <option key={s} value={s}>{s.replaceAll('_', ' ').toLowerCase()}</option>
            ))}
          </select>
        </Field>
      </div>

      {q.isLoading ? <Spinner /> : null}
      {q.error ? <ErrorNote>{msg(q.error)}</ErrorNote> : null}
      {q.data && q.data.length === 0 ? <p className="kb-empty">No applicants with this status.</p> : null}

      {q.data?.map((item) => (
        <KycCard key={item.user_id} item={item} onDone={() => qc.invalidateQueries({ queryKey: ['ops-kyc'] })} />
      ))}
    </>
  );
}

function KycCard({ item, onDone }: { item: KycItem; onDone: () => void }) {
  const api = useOpsApi();
  const [note, setNote] = useState('');

  const decide = useMutation({
    mutationFn: (action: 'approve' | 'reject') =>
      opsPost(api, `/v1/ops/kyc/${item.user_id}:${action}`, { kind: item.kind, note: note || undefined }),
    onSuccess: onDone,
  });

  return (
    <Card title={`${item.name || 'Unnamed'} · ${item.kind.toLowerCase()}`}>
      <div className="kb-row kb-between">
        <span className="kb-muted">User {item.user_id}</span>
        <Badge tone={item.status === 'APPROVED' ? 'good' : item.status === 'REJECTED' ? 'hazard' : 'warn'}>
          {item.status.replaceAll('_', ' ').toLowerCase()}
        </Badge>
      </div>
      {item.submitted_at ? <p className="kb-muted">Submitted {formatDateTime(item.submitted_at)}</p> : null}

      {item.documents.length === 0 ? (
        <p className="kb-empty">No documents uploaded.</p>
      ) : (
        <div className="kb-grid kb-grid-2">
          {item.documents.map((d) => <DocCard key={d.id} doc={d} />)}
        </div>
      )}

      <Field label="Review note (optional)">
        <input className="kb-input" value={note} onChange={(e) => setNote(e.target.value)} placeholder="Reason / remark" />
      </Field>
      {decide.error ? <ErrorNote>{msg(decide.error)}</ErrorNote> : null}
      <div className="kb-row">
        <Button onClick={() => decide.mutate('approve')} disabled={decide.isPending}>
          {decide.isPending ? 'Saving…' : 'Approve'}
        </Button>
        <Button variant="danger" onClick={() => decide.mutate('reject')} disabled={decide.isPending}>
          Reject
        </Button>
      </div>
    </Card>
  );
}

function DocCard({ doc }: { doc: KycDocument }) {
  const extracted = Object.entries(doc.extracted ?? {});
  return (
    <section className="kb-card">
      <div className="kb-between">
        <strong>{doc.kind.replaceAll('_', ' ')}</strong>
        {doc.check ? (
          <Badge tone={doc.check.outcome === 'VERIFIED' ? 'good' : doc.check.outcome === 'MISMATCH' ? 'hazard' : 'warn'}>
            {doc.check.outcome.replaceAll('_', ' ').toLowerCase()}
          </Badge>
        ) : null}
      </div>
      <p className="kb-muted">Uploaded {formatDate(doc.uploaded_at)}{doc.expires_at ? ` · expires ${formatDate(doc.expires_at)}` : ''}</p>
      {doc.check?.detail ? <p className="kb-muted">Check ({doc.check.provider}): {doc.check.detail}</p> : null}
      {extracted.length === 0 ? (
        <p className="kb-muted">No extracted fields.</p>
      ) : (
        <dl className="kb-list">
          {extracted.map(([k, v]) => (
            <div key={k} className="kb-between">
              <dt className="kb-muted">{k.replaceAll('_', ' ')}</dt>
              <dd><strong>{String(v)}</strong></dd>
            </div>
          ))}
        </dl>
      )}
    </section>
  );
}
