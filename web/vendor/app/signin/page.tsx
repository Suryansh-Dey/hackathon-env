'use client';
import { Button, Card, ErrorNote, Field, Spinner, apiBaseUrl, useApi, useAuth } from '@kabados/ui';
import { useRouter } from 'next/navigation';
import { useEffect, useState } from 'react';
import { pth } from '../lib/api';

export default function SignIn() {
  const { token, setToken } = useAuth();
  const api = useApi();
  const router = useRouter();
  const [ready, setReady] = useState(false);
  useEffect(() => setReady(true), []);

  const [phone, setPhone] = useState('+91');
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // Vendor onboarding (shown after sign-in when the account has no vendor role).
  const [me, setMe] = useState<{ hasVendor: boolean } | null>(null);
  const [biz, setBiz] = useState('');
  const [city, setCity] = useState('');
  const [lat, setLat] = useState('');
  const [lng, setLng] = useState('');
  const [cats, setCats] = useState('');

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
    } catch (e) {
      setErr(e instanceof Error ? e.message : 'Sign-in failed');
    } finally {
      setBusy(false);
    }
  }

  // Once signed in, check whether this account is already a vendor.
  useEffect(() => {
    if (!ready || !token) return;
    let active = true;
    (async () => {
      const { data } = await api.GET('/v1/me');
      if (!active) return;
      const hasVendor = !!data?.vendor || !!data?.roles?.includes('VENDOR');
      setMe({ hasVendor });
      if (data) {
        setCity((c) => c || data.vendor?.city || '');
        setBiz((b) => b || data.vendor?.business_name || '');
      }
      if (hasVendor) router.replace('/');
    })();
    return () => {
      active = false;
    };
  }, [ready, token, api, router]);

  async function onboard() {
    setErr(null);
    setBusy(true);
    try {
      const latN = Number(lat);
      const lngN = Number(lng);
      if (!biz.trim() || !city.trim() || Number.isNaN(latN) || Number.isNaN(lngN)) {
        throw new Error('Fill business name, city and yard coordinates.');
      }
      const { error } = await api.POST('/v1/me/roles/{role}:apply', {
        params: pth({ role: 'VENDOR' }),
        body: {
          business_name: biz.trim(),
          city: city.trim(),
          yard_location: { lat: latN, lng: lngN },
          categories: cats.split(',').map((c) => c.trim()).filter(Boolean),
        },
      });
      if (error) throw error;
      router.replace('/');
    } catch (e) {
      setErr(e instanceof Error ? e.message : 'Could not complete onboarding');
    } finally {
      setBusy(false);
    }
  }

  if (!ready) return <Spinner />;

  if (!token) {
    return (
      <Card title="Vendor sign in">
        <p className="kb-muted">Enter your mobile number to manage your yard, rate card and pickups.</p>
        <Field label="Mobile number">
          <input className="kb-input" value={phone} onChange={(e) => setPhone(e.target.value)} inputMode="tel" placeholder="+91XXXXXXXXXX" />
        </Field>
        {err ? <ErrorNote>{err}</ErrorNote> : null}
        <Button onClick={go} disabled={busy}>{busy ? 'Signing in…' : 'Continue'}</Button>
      </Card>
    );
  }

  if (!me) return <Spinner label="Checking your account…" />;

  // Signed in but not yet a vendor → onboarding.
  return (
    <Card title="Set up your yard">
      <p className="kb-muted">A few details unlock the vendor tools. You can refine your service area later in the rate card.</p>
      <Field label="Business name">
        <input className="kb-input" value={biz} onChange={(e) => setBiz(e.target.value)} placeholder="e.g. Sharma Scrap Traders" />
      </Field>
      <Field label="City">
        <input className="kb-input" value={city} onChange={(e) => setCity(e.target.value)} placeholder="e.g. Pune" />
      </Field>
      <div className="kb-grid-2">
        <Field label="Yard latitude">
          <input className="kb-input" value={lat} onChange={(e) => setLat(e.target.value)} inputMode="decimal" placeholder="18.5204" />
        </Field>
        <Field label="Yard longitude">
          <input className="kb-input" value={lng} onChange={(e) => setLng(e.target.value)} inputMode="decimal" placeholder="73.8567" />
        </Field>
      </div>
      <Field label="Categories you buy (comma-separated globs, optional)">
        <input className="kb-input" value={cats} onChange={(e) => setCats(e.target.value)} placeholder="metal.*, paper.*, ewaste.*" />
      </Field>
      {err ? <ErrorNote>{err}</ErrorNote> : null}
      <Button onClick={onboard} disabled={busy}>{busy ? 'Submitting…' : 'Become a vendor'}</Button>
    </Card>
  );
}
