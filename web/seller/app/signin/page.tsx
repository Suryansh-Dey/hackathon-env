'use client';
import { Button, Card, ErrorNote, Field, apiBaseUrl, useAuth } from '@kabados/ui';
import { useRouter } from 'next/navigation';
import { useState } from 'react';

export default function SignIn() {
  const { setToken } = useAuth();
  const router = useRouter();
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
      if (!res.ok) throw new Error('Could not sign in with that number');
      const data = await res.json();
      setToken(data.token, data.sub);
      router.push('/');
    } catch (e) {
      setErr(e instanceof Error ? e.message : 'Sign-in failed');
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
