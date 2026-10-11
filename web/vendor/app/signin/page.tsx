'use client';
import { Button, Card, ErrorNote, Field, Spinner, apiBaseUrl, useApi, useAuth } from '@kabados/ui';
import { useRouter } from 'next/navigation';
import { useEffect, useState } from 'react';
import { msg, pth } from '../lib/api';

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
  const [name, setName] = useState('');
  const [adult, setAdult] = useState(false);
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
      const data = await res.json().catch(() => null);
      if (!res.ok) throw data ?? new Error('Could not sign in with that number');
      setToken(data.token, data.sub);
    } catch (e) {
      setErr(msg(e, 'Sign-in failed'));
    } finally {
      setBusy(false);
    }
  }

  // Once signed in, check whether this account is already a vendor.
  useEffect(() => {
    if (!ready || !token) return;
    let active = true;
    (async () => {
      const { data, error } = await api.GET('/v1/me');
      if (!active) return;
      if (error) {
        setErr(msg(error, 'Could not load your account'));
        setMe({ hasVendor: false });
        return;
      }
      const hasVendor = !!data?.vendor || !!data?.roles?.includes('VENDOR');
      setMe({ hasVendor });
      if (data) {
        setName((n) => n || data.name || '');
        setAdult((a) => a || data.adult_confirmed);
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
      if (!name.trim()) throw new Error('Enter your name.');
      if (!adult) throw new Error('Confirm you are 18 or older to continue.');
      if (!biz.trim() || !city.trim() || !lat.trim() || !lng.trim() || Number.isNaN(latN) || Number.isNaN(lngN)) {
        throw new Error('Fill business name, city and yard coordinates.');
      }
      // Profile first: the server refuses role applications until the account
      // has a name and has confirmed 18+.
      const { error: profileErr } = await api.PATCH('/v1/me', { body: { name: name.trim(), adult_confirmed: true } });
      if (profileErr) throw profileErr;
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
      setErr(msg(e, 'Could not complete onboarding'));
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
      <Field label="Your name">
        <input className="kb-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. Ramesh Sharma" autoComplete="name" />
      </Field>
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
      <Field label="Categories you buy (exact category ids, comma-separated, optional)">
        <input className="kb-input" value={cats} onChange={(e) => setCats(e.target.value)} placeholder="ewaste.itew.laptop, ewaste.itew.mobile_phone" />
      </Field>
      <label style={{ display: 'flex', gap: 8, alignItems: 'center', margin: '8px 0', cursor: 'pointer' }}>
        <input type="checkbox" checked={adult} onChange={(e) => setAdult(e.target.checked)} /> I am 18 or older
      </label>
      {err ? <ErrorNote>{err}</ErrorNote> : null}
      <Button onClick={onboard} disabled={busy}>{busy ? 'Submitting…' : 'Become a vendor'}</Button>
    </Card>
  );
}
