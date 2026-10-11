'use client';
import { Badge, Button, Card, ErrorNote, Field, Spinner, formatDateTime } from '@kabados/ui';
import { useMutation, useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { StaffGate, msg, opsPost, useMounted, useOpsApi, type OpsConfig } from '../ops-api';

type MeView = { id: string; name?: string | null; roles: string[]; phone_last4: string };

export default function ConfigPage() {
  return (
    <StaffGate>
      <ConfigView />
      <SuspendTool />
    </StaffGate>
  );
}

function ConfigView() {
  const api = useOpsApi();
  const mounted = useMounted();
  const q = useQuery({
    queryKey: ['ops-config'],
    enabled: mounted,
    queryFn: async () => {
      const { data, error } = await api.GET('/v1/ops/config');
      if (error) throw error;
      return data as OpsConfig;
    },
  });

  if (q.isLoading) return <Spinner />;
  if (q.error) return <ErrorNote>{msg(q.error)}</ErrorNote>;
  const c = q.data;
  if (!c) return null;

  const modes = Object.entries(c.provider_modes ?? {});
  const rows: [string, string][] = [
    ['Environment', c.env],
    ['Platform fee', `${(c.platform_fee_bps / 100).toFixed(2)}% (${c.platform_fee_bps} bps)`],
    ['First-look window', `${c.first_look_minutes} min`],
    ['OTP enabled', c.otp_enabled ? 'yes' : 'no'],
    ['Checked at', formatDateTime(c.checked_at)],
  ];

  return (
    <Card title="Platform configuration">
      <table className="kb-list" style={{ width: '100%' }}>
        <tbody>
          {rows.map(([k, v]) => (
            <tr key={k}><td className="kb-muted">{k}</td><td style={{ textAlign: 'right' }}><strong>{v}</strong></td></tr>
          ))}
        </tbody>
      </table>
      <h3>Provider modes</h3>
      {modes.length === 0 ? <p className="kb-empty">None reported.</p> : (
        <table className="kb-list" style={{ width: '100%' }}>
          <tbody>
            {modes.map(([k, v]) => (
              <tr key={k}><td className="kb-muted">{k}</td><td style={{ textAlign: 'right' }}><strong>{String(v)}</strong></td></tr>
            ))}
          </tbody>
        </table>
      )}
    </Card>
  );
}

function SuspendTool() {
  const api = useOpsApi();
  const [userId, setUserId] = useState('');

  const act = useMutation({
    mutationFn: (action: 'suspend' | 'unsuspend') => opsPost<MeView>(api, `/v1/ops/users/${userId}:${action}`),
  });

  return (
    <Card title="Suspend / unsuspend user">
      <Field label="User ID">
        <input className="kb-input" value={userId} onChange={(e) => setUserId(e.target.value)} placeholder="usr_…" />
      </Field>
      {act.error ? <ErrorNote>{msg(act.error)}</ErrorNote> : null}
      {act.data ? (
        <p className="kb-muted">
          Updated {act.data.name || act.data.id} · roles {act.data.roles.join(', ') || '—'}
        </p>
      ) : null}
      <div className="kb-row">
        <Button variant="danger" onClick={() => act.mutate('suspend')} disabled={act.isPending || !userId}>
          {act.isPending ? 'Working…' : 'Suspend'}
        </Button>
        <Button onClick={() => act.mutate('unsuspend')} disabled={act.isPending || !userId}>
          Unsuspend
        </Button>
      </div>
      <p className="kb-muted"><Badge tone="warn">caution</Badge> Suspension blocks the user from all actions immediately.</p>
    </Card>
  );
}
