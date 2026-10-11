'use client';
import { Button, Card, ErrorNote, Field, apiBaseUrl, useAuth } from '@kabados/ui';
import { useRouter } from 'next/navigation';
import { useState } from 'react';
import { msg } from '../ops-api';

// Staff dev sign-in: phone + staff flag + admin key, no OTP
// (POST /dev/token { phone, staff: true, admin_key }); a wrong key is a 403.
// Returns a staff bearer token used by every /v1/ops endpoint. Prod would use Cognito staff groups.
export default function StaffSignIn() {
  const { setToken } = useAuth();
  const router = useRouter();
  const [phone, setPhone] = useState('+91');
  const [adminKey, setAdminKey] = useState('');
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function go() {
    setErr(null);
    setBusy(true);
    try {
      const res = await fetch(`${apiBaseUrl()}/dev/token`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ phone, staff: true, admin_key: adminKey }),
      });
      const data = await res.json().catch(() => null);
      if (res.status === 403) throw new Error(data?.detail || 'That admin key is not correct.');
      if (!res.ok) throw data ?? new Error('Could not sign in as staff with that number');
      setToken(data.token, data.sub);
      router.push('/kyc/');
    } catch (e) {
      setErr(msg(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card title="Staff sign in">
      <p className="kb-muted">Enter your staff mobile number and the admin key to open the ops console.</p>
      <Field label="Mobile number">
        <input
          className="kb-input"
          value={phone}
          onChange={(e) => setPhone(e.target.value)}
          inputMode="tel"
          placeholder="+91XXXXXXXXXX"
        />
      </Field>
      <Field label="Admin key">
        <input
          className="kb-input"
          type="password"
          value={adminKey}
          onChange={(e) => setAdminKey(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter' && adminKey) go(); }}
          autoComplete="current-password"
        />
      </Field>
      {err ? <ErrorNote>{err}</ErrorNote> : null}
      <Button onClick={go} disabled={busy || !adminKey}>{busy ? 'Signing in…' : 'Continue'}</Button>
    </Card>
  );
}
