'use client';
import { Button, Card, ErrorNote, Field, apiBaseUrl, problemMessage, useAuth } from '@kabados/ui';
import { useState } from 'react';

export default function SignIn() {
  const { setToken } = useAuth();
  const [phone, setPhone] = useState('+91');
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // Dev sign-in: phone only, no OTP (POST /dev/token). Prod would use Cognito.
  async function go() {
    setErr(null);
    setBusy(true);
    try {
      const res = await fetch(`${apiBaseUrl()}/dev/token`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ phone }),
      });
      const data = await res.json().catch(() => null);
      if (!res.ok) throw data ?? new Error('Could not sign in with that number');
      setToken(data.token, data.sub);
      // Full reload so no cached data from a previous account survives.
      window.location.assign('/');
    } catch (e) {
      setErr(problemMessage(e, 'Could not sign in with that number'));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card title="Sign in">
      <p className="kb-muted">Enter your mobile number to continue.</p>
      <Field label="Mobile number">
        <input className="kb-input" value={phone} onChange={(e) => setPhone(e.target.value)} inputMode="tel" placeholder="+91XXXXXXXXXX" />
      </Field>
      {err ? <ErrorNote>{err}</ErrorNote> : null}
      <Button onClick={go} disabled={busy}>{busy ? 'Signing in…' : 'Continue'}</Button>
    </Card>
  );
}
