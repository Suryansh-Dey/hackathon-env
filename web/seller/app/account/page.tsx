'use client';
import { Badge, Button, Card, ErrorNote, Field, Spinner, problemMessage, useApi, useAuth } from '@kabados/ui';
import { useMutation, useQuery } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import { ProfileForm, useMe } from '../profile';

const VALIDATION: Record<string, { text: string; tone: 'good' | 'warn' | 'hazard' }> = {
  VALIDATED: { text: 'Verified', tone: 'good' },
  PENDING: { text: 'Checking', tone: 'warn' },
  FAILED: { text: 'Could not verify', tone: 'hazard' },
};

export default function Account() {
  const { token } = useAuth();
  const api = useApi();
  const me = useMe();
  const [ready, setReady] = useState(false);
  useEffect(() => setReady(true), []);

  const [vpa, setVpa] = useState('');
  const [exportUrl, setExportUrl] = useState<string | null>(null);
  const [exportJson, setExportJson] = useState<string | null>(null);

  const payouts = useQuery({
    queryKey: ['payout-methods'],
    enabled: ready && !!token,
    queryFn: async () => {
      const { data, error } = await api.GET('/v1/me/payout-methods');
      if (error) throw error;
      return data;
    },
  });

  const addresses = useQuery({
    queryKey: ['addresses'],
    enabled: ready && !!token,
    queryFn: async () => {
      const { data, error } = await api.GET('/v1/me/addresses');
      if (error) throw error;
      return data;
    },
  });

  const addPayout = useMutation({
    mutationFn: async () => {
      const v = vpa.trim();
      if (!/^[^@\s]+@[^@\s]+$/.test(v)) throw new Error('Enter a valid UPI ID, like name@bank');
      const { data, error } = await api.POST('/v1/me/payout-methods', { body: { vpa: v } });
      if (error) throw error;
      return data;
    },
    onSuccess: () => {
      setVpa('');
      payouts.refetch();
    },
  });

  const dataExport = useMutation({
    mutationFn: async () => {
      const { data, error } = await api.POST('/v1/me/data-export');
      if (error) throw error;
      return data;
    },
    onSuccess: (data) => {
      const json = JSON.stringify(data, null, 2);
      if (exportUrl) URL.revokeObjectURL(exportUrl);
      const url = URL.createObjectURL(new Blob([json], { type: 'application/json' }));
      setExportUrl(url);
      setExportJson(json);
      // Start the download right away; the button below stays as a fallback.
      const a = document.createElement('a');
      a.href = url;
      a.download = 'kabados-my-data.json';
      a.click();
    },
  });

  if (!ready) return <Spinner />;
  if (!token)
    return (
      <Card title="Sign in">
        <p className="kb-muted">Sign in to manage your account.</p>
        <a href="/signin/"><Button>Sign in</Button></a>
      </Card>
    );

  const hasValidUpi = (payouts.data ?? []).some((p) => p.validation === 'VALIDATED');

  return (
    <>
      <h1 className="kb-h1">Account</h1>
      {me.data ? <p className="kb-muted">Signed in with the number ending {me.data.phone_last4}.</p> : null}

      <ProfileForm title="Profile" />

      <Card title="Get paid by UPI">
        <p className="kb-muted">Offers paid through KabadOS are sent to this UPI ID after pickup. You need one before accepting such an offer.</p>
        {payouts.isLoading ? <Spinner /> : null}
        {payouts.error ? <ErrorNote>{problemMessage(payouts.error, 'Could not load your payout methods')}</ErrorNote> : null}
        {payouts.data && payouts.data.length > 0 ? (
          <ul className="kb-list">
            {payouts.data.map((p) => {
              const v = VALIDATION[p.validation] ?? { text: p.validation, tone: 'warn' as const };
              return (
                <li key={p.id} className="kb-between">
                  <span>
                    <strong>{p.display}</strong>
                    {p.validated_name ? <><br /><span className="kb-muted">{p.validated_name}</span></> : null}
                  </span>
                  <Badge tone={v.tone}>{v.text}</Badge>
                </li>
              );
            })}
          </ul>
        ) : payouts.data ? (
          <p className="kb-muted">No UPI ID yet.</p>
        ) : null}
        <Field label={hasValidUpi ? 'Add another UPI ID' : 'UPI ID'}>
          <input className="kb-input" value={vpa} onChange={(e) => setVpa(e.target.value)} placeholder="name@bank" autoCapitalize="none" autoCorrect="off" spellCheck={false} />
        </Field>
        {addPayout.error ? <ErrorNote>{problemMessage(addPayout.error, 'Could not add that UPI ID')}</ErrorNote> : null}
        <Button onClick={() => addPayout.mutate()} disabled={addPayout.isPending || !vpa.trim()}>{addPayout.isPending ? 'Verifying…' : 'Add UPI ID'}</Button>
      </Card>

      <Card title="Saved addresses">
        {addresses.isLoading ? <Spinner /> : null}
        {addresses.error ? <ErrorNote>{problemMessage(addresses.error, 'Could not load your addresses')}</ErrorNote> : null}
        {addresses.data && addresses.data.length === 0 ? <p className="kb-muted">No saved addresses yet. You can add one when you <a href="/new/">sell scrap</a>.</p> : null}
        <ul className="kb-list">
          {addresses.data?.map((a) => (
            <li key={a.id}>
              <strong>{a.label}</strong> — {a.text}
              <br />
              <span className="kb-muted">{a.landmark ? `${a.landmark} · ` : ''}{a.city}</span>
            </li>
          ))}
        </ul>
      </Card>

      <Card title="Your data">
        <p className="kb-muted">Download a copy of everything KabadOS stores about you: profile, addresses, consents, listings and pickups.</p>
        {dataExport.error ? <ErrorNote>{problemMessage(dataExport.error, 'Could not export your data')}</ErrorNote> : null}
        <div className="kb-row">
          <Button variant="ghost" onClick={() => dataExport.mutate()} disabled={dataExport.isPending}>{dataExport.isPending ? 'Preparing…' : 'Download my data'}</Button>
          {exportUrl ? <a href={exportUrl} download="kabados-my-data.json"><Button>Save kabados-my-data.json</Button></a> : null}
        </div>
        {exportJson ? (
          <details style={{ marginTop: 12 }}>
            <summary>View the data</summary>
            <pre style={{ whiteSpace: 'pre-wrap', wordBreak: 'break-word', fontSize: '0.8rem', maxHeight: 400, overflow: 'auto' }}>{exportJson}</pre>
          </details>
        ) : null}
      </Card>
    </>
  );
}
