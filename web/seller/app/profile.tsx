'use client';
// The caller's profile (GET /v1/me) and the one-time "name + 18+" step every
// new account must finish before it can list scrap (PLAN.md §5.3).
import { Button, Card, ErrorNote, Field, Spinner, problemMessage, useApi, useAuth } from '@kabados/ui';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useState, type ReactNode } from 'react';

export function useMe() {
  const { token } = useAuth();
  const api = useApi();
  return useQuery({
    queryKey: ['me'],
    enabled: !!token,
    queryFn: async () => {
      const { data, error } = await api.GET('/v1/me');
      if (error) throw error;
      return data;
    },
  });
}

/** Name + "I am 18 or older" form; PATCHes /v1/me. */
export function ProfileForm({ title = 'About you', intro, onSaved }: { title?: string; intro?: string; onSaved?: () => void }) {
  const api = useApi();
  const qc = useQueryClient();
  const me = useMe();
  const [name, setName] = useState('');
  const [adult, setAdult] = useState(false);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    if (me.data) {
      setName(me.data.name ?? '');
      setAdult(me.data.adult_confirmed);
    }
  }, [me.data]);

  const save = useMutation({
    mutationFn: async () => {
      setSaved(false);
      if (!name.trim()) throw new Error('Enter your name');
      if (!adult) throw new Error('KabadOS is for adults (18+) only. Tick the box to continue.');
      const { data, error } = await api.PATCH('/v1/me', { body: { name: name.trim(), adult_confirmed: true } });
      if (error) throw error;
      return data;
    },
    onSuccess: (data) => {
      qc.setQueryData(['me'], data);
      setSaved(true);
      onSaved?.();
    },
  });

  return (
    <Card title={title}>
      {intro ? <p className="kb-muted">{intro}</p> : null}
      <Field label="Your name">
        <input className="kb-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="Full name" autoComplete="name" maxLength={80} />
      </Field>
      <label className="kb-row" style={{ marginBottom: 14, cursor: 'pointer' }}>
        <input type="checkbox" checked={adult} onChange={(e) => setAdult(e.target.checked)} disabled={me.data?.adult_confirmed} />
        <span>I am 18 or older</span>
      </label>
      {save.error ? <ErrorNote>{problemMessage(save.error, 'Could not save your profile')}</ErrorNote> : null}
      {saved && !save.isPending ? <p className="kb-muted">Saved.</p> : null}
      <Button onClick={() => save.mutate()} disabled={save.isPending}>{save.isPending ? 'Saving…' : 'Save'}</Button>
    </Card>
  );
}

/** Shows the profile step instead of `children` until the account has a name and is 18+ confirmed. */
export function ProfileGate({ children }: { children: ReactNode }) {
  const me = useMe();
  if (me.isLoading) return <Spinner />;
  if (me.error) return <ErrorNote>{problemMessage(me.error, 'Could not load your profile')}</ErrorNote>;
  if (me.data && (!me.data.adult_confirmed || !me.data.name))
    return <ProfileForm title="Welcome! Tell us about you" intro="Add your name and confirm you are 18 or older to start selling. You only do this once." />;
  return <>{children}</>;
}
