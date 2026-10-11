'use client';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Badge, Button, Card, ErrorNote, Field, Spinner, formatPaise, stars, useApi, useAuth } from '@kabados/ui';
import { useEffect, useState } from 'react';
import { msg, pth, qry } from '../lib/api';

type Dispatch = 'SEQUENTIAL' | 'BROADCAST';

function eta(s?: number | null): string | null {
  if (s == null) return null;
  const m = Math.round(s / 60);
  return m < 60 ? `${m} min away` : `${(m / 60).toFixed(1)} h away`;
}

export default function Transporters() {
  const { token } = useAuth();
  const api = useApi();
  const qc = useQueryClient();
  const [ready, setReady] = useState(false);
  const [dealId, setDealId] = useState<string | null>(null);
  useEffect(() => {
    setReady(true);
    try {
      setDealId(new URLSearchParams(window.location.search).get('deal_id'));
    } catch {
      /* SSR */
    }
  }, []);

  const list = useQuery({
    queryKey: ['transporters', dealId],
    enabled: ready && !!token,
    queryFn: async () => {
      const q = dealId ? qry({ deal_id: dealId }) : qry({});
      const { data, error } = await api.GET('/v1/vendor/transporters', { params: q });
      if (error) throw error;
      return data;
    },
  });

  // Invite
  const [phone, setPhone] = useState('+91');
  const [name, setName] = useState('');
  const [inviteFare, setInviteFare] = useState('');
  const invite = useMutation({
    mutationFn: async () => {
      const body: { phone: string; display_name: string; fare?: { type: 'FIXED'; amount_paise: number } } = {
        phone: phone.trim(),
        display_name: name.trim(),
      };
      const rupees = Number(inviteFare);
      if (!Number.isNaN(rupees) && rupees > 0) body.fare = { type: 'FIXED', amount_paise: Math.round(rupees * 100) };
      const { error } = await api.POST('/v1/vendor/transporters:invite', { params: {} as never, body });
      if (error) throw error;
    },
    onSuccess: () => {
      setName('');
      setInviteFare('');
      qc.invalidateQueries({ queryKey: ['transporters'] });
    },
  });

  // Assign job (needs a deal id in the link)
  const [selected, setSelected] = useState<string[]>([]);
  const [jobFare, setJobFare] = useState('');
  const [dispatch, setDispatch] = useState<Dispatch>('SEQUENTIAL');
  const [timeout, setTimeoutSecs] = useState('120');
  const assign = useMutation({
    mutationFn: async () => {
      const rupees = Number(jobFare);
      if (Number.isNaN(rupees) || rupees <= 0) throw new Error('Set a fare');
      if (selected.length === 0) throw new Error('Pick at least one transporter');
      const body: { dispatch: Dispatch; fare: number; transporter_ids: string[]; accept_timeout_secs?: number } = {
        dispatch,
        fare: Math.round(rupees * 100),
        transporter_ids: selected,
      };
      const t = Number(timeout);
      if (!Number.isNaN(t) && t > 0) body.accept_timeout_secs = t;
      const { error } = await api.POST('/v1/deals/{id}/jobs', { params: pth({ id: dealId! }), body });
      if (error) throw error;
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: ['deal', dealId] }),
  });

  const toggle = (tid: string) => setSelected((s) => (s.includes(tid) ? s.filter((x) => x !== tid) : [...s, tid]));

  if (!ready) return <Spinner />;
  if (!token)
    return <Card title="Sign in required"><a href="/signin/"><Button>Sign in</Button></a></Card>;

  return (
    <>
      <h1 className="kb-h1">Transporters</h1>

      {dealId ? (
        <Card title="Assign a pickup job">
          <p className="kb-muted"><a href={`/deal/?id=${encodeURIComponent(dealId)}`}>← Back to the deal</a></p>
          <p className="kb-muted">Offer deal <code>{dealId}</code> to your transporters. Sequential offers go down the list on timeout; broadcast asks everyone at once.</p>
          <div className="kb-grid-2">
            <Field label="Fare (₹)">
              <input className="kb-input" value={jobFare} onChange={(e) => setJobFare(e.target.value)} inputMode="numeric" placeholder="e.g. 250" />
            </Field>
            <Field label="Dispatch">
              <select className="kb-input" value={dispatch} onChange={(e) => setDispatch(e.target.value as Dispatch)}>
                <option value="SEQUENTIAL">Sequential</option>
                <option value="BROADCAST">Broadcast</option>
              </select>
            </Field>
          </div>
          <Field label="Accept timeout (seconds)">
            <input className="kb-input" value={timeout} onChange={(e) => setTimeoutSecs(e.target.value)} inputMode="numeric" placeholder="120" style={{ maxWidth: 160 }} />
          </Field>
          {selected.length ? <p className="kb-muted">{selected.length} transporter{selected.length === 1 ? '' : 's'} selected (tick them in the list below).</p> : null}
          {assign.error ? <ErrorNote>{msg(assign.error, 'Could not create the job.')}</ErrorNote> : null}
          {assign.isSuccess ? <p className="kb-muted">Job offered. <a href={`/deal/?id=${encodeURIComponent(dealId)}`}>Track it on the deal →</a></p> : null}
          <Button disabled={assign.isPending} onClick={() => assign.mutate()}>{assign.isPending ? 'Offering…' : 'Offer job'}</Button>
        </Card>
      ) : null}

      <Card title="Invite a transporter">
        <p className="kb-muted">Add a transporter who has signed up in the app to your trusted list.</p>
        <div className="kb-grid-2">
          <Field label="Phone">
            <input className="kb-input" value={phone} onChange={(e) => setPhone(e.target.value)} inputMode="tel" placeholder="+91XXXXXXXXXX" />
          </Field>
          <Field label="Display name">
            <input className="kb-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. Ravi (Tempo)" />
          </Field>
        </div>
        <Field label="Default fare ₹ (optional)">
          <input className="kb-input" value={inviteFare} onChange={(e) => setInviteFare(e.target.value)} inputMode="numeric" placeholder="e.g. 250" style={{ maxWidth: 160 }} />
        </Field>
        {invite.error ? <ErrorNote>{msg(invite.error, 'Could not send the invite. Check the number is registered.')}</ErrorNote> : null}
        {invite.isSuccess ? <p className="kb-muted">Invite sent.</p> : null}
        <Button disabled={invite.isPending || !phone.trim() || !name.trim()} onClick={() => invite.mutate()}>{invite.isPending ? 'Inviting…' : 'Invite'}</Button>
      </Card>

      <h2 className="kb-card-title">My transporters</h2>
      {list.isLoading ? <Spinner label="Loading…" /> : null}
      {list.error ? <ErrorNote>{msg(list.error, 'Could not load your transporters.')}</ErrorNote> : null}
      {list.data && list.data.length === 0 ? <p className="kb-empty">No transporters yet. Invite one above.</p> : null}
      <ul className="kb-list">
        {list.data?.map((t) => (
          <li key={t.transporter_id}>
            <div className="kb-between">
              <span>
                {dealId ? (
                  <label style={{ cursor: 'pointer' }}>
                    <input type="checkbox" checked={selected.includes(t.transporter_id)} onChange={() => toggle(t.transporter_id)} />{' '}
                    <strong>{t.display_name}</strong>
                  </label>
                ) : (
                  <strong>{t.display_name}</strong>
                )}
                <br />
                <span className="kb-muted">
                  {stars(t.rating)} · {Math.round(t.acceptance * 100)}% accept
                  {eta(t.eta_s) ? ` · ${eta(t.eta_s)}` : ''}
                  {t.fare && t.fare.type === 'FIXED' ? ` · ${formatPaise(t.fare.amount_paise)}` : ''}
                </span>
              </span>
              <Badge tone={t.on_duty ? 'good' : 'neutral'}>{t.on_duty ? 'On duty' : 'Off duty'}</Badge>
            </div>
          </li>
        ))}
      </ul>
    </>
  );
}
