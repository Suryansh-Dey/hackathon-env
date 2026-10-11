'use client';
import { Button, Card, ErrorNote, Field, apiBaseUrl, useAuth } from '@kabados/ui';
import { useRouter } from 'next/navigation';
import { useState } from 'react';

// Staff dev sign-in: phone + staff flag, no OTP (POST /dev/token { phone, staff: true }).
// Returns a staff bearer token used by every /v1/ops endpoint. Prod would use Cognito staff groups.
export default function StaffSignIn() {
  const { setToken } = useAuth();
  const router = useRouter();
  const [phone, setPhone] = useState('+91');
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function go() {
    setErr(null);
    setBusy(true);
    try {
      const res = await fetch(`${apiBaseUrl()}/dev/token`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ phone, staff: true }),
      });
      if (!res.ok) throw new Error('Could not sign in as staff with that number');
      const data = await res.json();
      setToken(data.token, data.sub);
      router.push('/kyc/');
    } catch (e) {
      setErr(e instanceof Error ? e.message : 'Sign-in failed');
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card title="Staff sign in">
      <p className="kb-muted">Enter your staff mobile number to open the ops console.</p>
      <Field label="Mobile number">
        <input
          className="kb-input"
          value={phone}
          onChange={(e) => setPhone(e.target.value)}
          inputMode="tel"
          placeholder="+91XXXXXXXXXX"
        />
      </Field>
      {err ? <ErrorNote>{err}</ErrorNote> : null}
      <Button onClick={go} disabled={busy}>{busy ? 'Signing in…' : 'Continue'}</Button>
    </Card>
  );
}
