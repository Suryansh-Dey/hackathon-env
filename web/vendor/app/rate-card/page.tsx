'use client';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Badge, Button, Card, ErrorNote, Field, Spinner, formatPaise, useApi, useAuth } from '@kabados/ui';
import { useEffect, useState } from 'react';
import { pth, type VersionPath } from '../lib/api';

type Weekday = 'MON' | 'TUE' | 'WED' | 'THU' | 'FRI' | 'SAT' | 'SUN';
const WEEKDAYS: Weekday[] = ['MON', 'TUE', 'WED', 'THU', 'FRI', 'SAT', 'SUN'];

type Issue = { message: string; path: string };
type RuleLineForm = { category_id: string; grade: string; unit: 'KG' | 'PIECE'; price: string; min_weight_kg: string };

function IssueList({ issues }: { issues: Issue[] }) {
  if (!issues.length) return null;
  return (
    <div className="kb-error" role="alert">
      <strong>Validation issues</strong>
      <ul style={{ margin: '4px 0 0', paddingLeft: 18 }}>
        {issues.map((i, n) => <li key={n}>{i.message}{i.path ? ` (${i.path})` : ''}</li>)}
      </ul>
    </div>
  );
}

export default function RateCard() {
  const { token } = useAuth();
  const api = useApi();
  const qc = useQueryClient();
  const [ready, setReady] = useState(false);
  useEffect(() => setReady(true), []);
  const enabled = ready && !!token;

  const ruleSets = useQuery({
    queryKey: ['rule-sets'],
    enabled,
    queryFn: async () => {
      const { data, error } = await api.GET('/v1/vendor/rule-sets');
      if (error) throw error;
      return data;
    },
  });

  const invalidate = () => qc.invalidateQueries({ queryKey: ['rule-sets'] });

  // (a) Draft from text
  const [instruction, setInstruction] = useState('');
  const draftFromText = useMutation({
    mutationFn: async () => {
      const { data, error } = await api.POST('/v1/vendor/rule-sets:draft-from-text', { params: {} as never, body: { instruction: instruction.trim() } });
      if (error) throw error;
      return data;
    },
    onSuccess: invalidate,
  });

  // (b) Bulk change
  const [glob, setGlob] = useState('');
  const [percent, setPercent] = useState('');
  const bulk = useMutation({
    mutationFn: async () => {
      const { data, error } = await api.POST('/v1/vendor/rule-sets:bulk-change', {
        params: {} as never,
        body: { category_glob: glob.trim(), percent: Number(percent) },
      });
      if (error) throw error;
      return data;
    },
    onSuccess: invalidate,
  });

  // (c) Form editor
  const [lat, setLat] = useState('');
  const [lng, setLng] = useState('');
  const [radiusKm, setRadiusKm] = useState('8');
  const [days, setDays] = useState<Weekday[]>(['MON', 'TUE', 'WED', 'THU', 'FRI', 'SAT']);
  const [slots, setSlots] = useState('10:00-18:00');
  const [capacityKg, setCapacityKg] = useState('');
  const [minOrder, setMinOrder] = useState('');
  const [minWeightKg, setMinWeightKg] = useState('');
  const [lines, setLines] = useState<RuleLineForm[]>([{ category_id: '', grade: '', unit: 'KG', price: '', min_weight_kg: '' }]);
  const [exclusions, setExclusions] = useState('');

  const toggleDay = (d: Weekday) => setDays((v) => (v.includes(d) ? v.filter((x) => x !== d) : [...v, d]));
  const setLine = (i: number, patch: Partial<RuleLineForm>) => setLines((ls) => ls.map((l, n) => (n === i ? { ...l, ...patch } : l)));
  const addLine = () => setLines((ls) => [...ls, { category_id: '', grade: '', unit: 'KG', price: '', min_weight_kg: '' }]);
  const removeLine = (i: number) => setLines((ls) => ls.filter((_, n) => n !== i));

  const saveForm = useMutation({
    mutationFn: async () => {
      const latN = Number(lat), lngN = Number(lng), rk = Number(radiusKm);
      if (Number.isNaN(latN) || Number.isNaN(lngN) || Number.isNaN(rk)) throw new Error('Set a valid service-area centre and radius.');
      const builtLines = lines
        .filter((l) => l.category_id.trim() && l.price.trim())
        .map((l) => {
          const paise = Math.round(Number(l.price) * 100);
          const base: Record<string, unknown> = { category_id: l.category_id.trim(), unit: l.unit };
          if (l.grade.trim()) base.grade = l.grade.trim();
          if (l.unit === 'KG') base.price_per_kg_paise = paise;
          else base.price_per_piece_paise = paise;
          if (l.min_weight_kg.trim()) base.min_weight_g = Math.round(Number(l.min_weight_kg) * 1000);
          return base;
        });
      if (!builtLines.length) throw new Error('Add at least one priced line.');
      const minimums: { order_value_paise?: number; weight_g?: number } = {};
      if (minOrder.trim()) minimums.order_value_paise = Math.round(Number(minOrder) * 100);
      if (minWeightKg.trim()) minimums.weight_g = Math.round(Number(minWeightKg) * 1000);
      const schedule: { days: Weekday[]; slots: string[]; daily_capacity_kg?: number } = {
        days,
        slots: slots.split(',').map((s) => s.trim()).filter(Boolean),
      };
      if (capacityKg.trim()) schedule.daily_capacity_kg = Number(capacityKg);
      const body = {
        service_area: { type: 'RADIUS' as const, center: { lat: latN, lng: lngN }, radius_km: rk },
        schedule,
        minimums,
        lines: builtLines,
        exclusions: exclusions.split(',').map((s) => s.trim()).filter(Boolean),
      };
      const { data, error } = await api.POST('/v1/vendor/rule-sets', { params: {} as never, body: body as never });
      if (error) throw error;
      return data;
    },
    onSuccess: invalidate,
  });

  // Simulate / Publish
  const simulate = useMutation({
    mutationFn: async (vars: { id: string; v: number }) => {
      const { error } = await api.POST('/v1/vendor/rule-sets/{id}/versions/{v}:simulate' as VersionPath, {
        params: pth({ id: vars.id, v: String(vars.v) }),
      });
      if (error) throw error;
    },
    onSuccess: invalidate,
  });
  const publish = useMutation({
    mutationFn: async (vars: { id: string; v: number }) => {
      const { error } = await api.POST('/v1/vendor/rule-sets/{id}/versions/{v}:publish', {
        params: pth({ id: vars.id, v: String(vars.v) }),
      });
      if (error) throw error;
    },
    onSuccess: invalidate,
  });

  if (!ready) return <Spinner />;
  if (!token)
    return <Card title="Sign in required"><a href="/signin/"><Button>Sign in</Button></a></Card>;
  if (ruleSets.isLoading) return <Spinner label="Loading rate card…" />;

  const draft = ruleSets.data?.draft ?? null;
  const draftIssues = (ruleSets.data?.draft_issues ?? []) as Issue[];
  const published = ruleSets.data?.published ?? [];

  const textResult = draftFromText.data;
  const bulkResult = bulk.data;

  return (
    <>
      <h1 className="kb-h1">Rate card</h1>
      <p className="kb-muted">Edit by typing, by bulk change, or with the full form. Review the draft, simulate it, then publish. Nothing goes live until you publish, and a draft with issues can’t be published.</p>

      {/* Current draft */}
      <Card title="Current draft">
        {draft ? (
          <>
            <div className="kb-between">
              <span>v{draft.version} · {draft.lines.length} priced line{draft.lines.length === 1 ? '' : 's'}</span>
              <Badge tone={draftIssues.length ? 'warn' : 'good'}>{draftIssues.length ? `${draftIssues.length} issue${draftIssues.length === 1 ? '' : 's'}` : 'Ready'}</Badge>
            </div>
            <ul className="kb-list">
              {draft.lines.map((l, i) => (
                <li key={i}>
                  <strong>{l.category_id}</strong>{l.grade ? ` · ${l.grade}` : ''}{' — '}
                  {l.price_per_kg_paise != null ? `${formatPaise(l.price_per_kg_paise)}/kg` : l.price_per_piece_paise != null ? `${formatPaise(l.price_per_piece_paise)}/pc` : '—'}
                  {l.min_weight_g ? <span className="kb-muted"> · min {(l.min_weight_g / 1000).toFixed(1)} kg</span> : null}
                </li>
              ))}
            </ul>
            <IssueList issues={draftIssues} />
            <div className="kb-row" style={{ gap: 8 }}>
              <Button variant="ghost" disabled={simulate.isPending} onClick={() => simulate.mutate({ id: draft.rule_set_id, v: draft.version })}>
                {simulate.isPending ? 'Simulating…' : 'Simulate'}
              </Button>
              <Button disabled={publish.isPending || draftIssues.length > 0} onClick={() => publish.mutate({ id: draft.rule_set_id, v: draft.version })}>
                {publish.isPending ? 'Publishing…' : 'Publish'}
              </Button>
            </div>
            {simulate.isSuccess ? <p className="kb-muted">Simulation ran. Review issues above before publishing.</p> : null}
            {simulate.error ? <ErrorNote>Simulation failed — fix the issues and retry.</ErrorNote> : null}
            {publish.error ? <ErrorNote>Could not publish. A draft with issues can’t go live.</ErrorNote> : null}
            {publish.isSuccess ? <p className="kb-muted">Published. It’s now your live rate card.</p> : null}
          </>
        ) : (
          <p className="kb-muted">No draft yet. Use one of the editors below to create one.</p>
        )}
      </Card>

      {/* (a) From text */}
      <Card title="Draft from text">
        <p className="kb-muted">e.g. “copper 650, brass 400, paper 14 above 5 kilo, no glass”.</p>
        <Field label="Instruction">
          <textarea className="kb-input" rows={3} value={instruction} onChange={(e) => setInstruction(e.target.value)} />
        </Field>
        <Button disabled={draftFromText.isPending || !instruction.trim()} onClick={() => draftFromText.mutate()}>
          {draftFromText.isPending ? 'Drafting…' : 'Draft from text'}
        </Button>
        {draftFromText.error ? <ErrorNote>Could not draft from that instruction.</ErrorNote> : null}
        {textResult ? (
          <div style={{ marginTop: 12 }}>
            <p className="kb-muted">Drafted {textResult.draft.lines.length} line(s).</p>
            {textResult.ambiguities.length ? (
              <><strong>Questions to confirm</strong><ul className="kb-list">{textResult.ambiguities.map((a, i) => <li key={i} className="kb-muted">{a}</li>)}</ul></>
            ) : null}
            {textResult.far_from_index.length ? (
              <><strong>Far from Rate Index (±40%)</strong><ul className="kb-list">{textResult.far_from_index.map((a, i) => <li key={i} className="kb-muted">{a}</li>)}</ul></>
            ) : null}
            <IssueList issues={textResult.issues as Issue[]} />
          </div>
        ) : null}
      </Card>

      {/* (b) Bulk change */}
      <Card title="Bulk change">
        <p className="kb-muted">e.g. all metals +3%.</p>
        <div className="kb-grid-2">
          <Field label="Category glob">
            <input className="kb-input" value={glob} onChange={(e) => setGlob(e.target.value)} placeholder="metal.*" />
          </Field>
          <Field label="Percent change">
            <input className="kb-input" value={percent} onChange={(e) => setPercent(e.target.value)} inputMode="numeric" placeholder="3 or -5" />
          </Field>
        </div>
        <Button disabled={bulk.isPending || !glob.trim() || !percent.trim()} onClick={() => bulk.mutate()}>
          {bulk.isPending ? 'Applying…' : 'Apply bulk change'}
        </Button>
        {bulk.error ? <ErrorNote>Could not apply the bulk change.</ErrorNote> : null}
        {bulkResult ? (
          <div style={{ marginTop: 12 }}>
            <p className="kb-muted">Updated draft has {bulkResult.draft.lines.length} line(s).</p>
            <IssueList issues={bulkResult.issues as Issue[]} />
          </div>
        ) : null}
      </Card>

      {/* (c) Form editor */}
      <Card title="Full form editor">
        <h3 className="kb-card-title">Service area</h3>
        <div className="kb-grid-2">
          <Field label="Centre latitude"><input className="kb-input" value={lat} onChange={(e) => setLat(e.target.value)} inputMode="decimal" placeholder="18.5204" /></Field>
          <Field label="Centre longitude"><input className="kb-input" value={lng} onChange={(e) => setLng(e.target.value)} inputMode="decimal" placeholder="73.8567" /></Field>
        </div>
        <Field label="Radius (km)"><input className="kb-input" value={radiusKm} onChange={(e) => setRadiusKm(e.target.value)} inputMode="decimal" style={{ maxWidth: 160 }} /></Field>

        <h3 className="kb-card-title">Schedule</h3>
        <div className="kb-row" style={{ gap: 6, flexWrap: 'wrap' }}>
          {WEEKDAYS.map((d) => (
            <label key={d} style={{ cursor: 'pointer' }}>
              <input type="checkbox" checked={days.includes(d)} onChange={() => toggleDay(d)} /> {d}
            </label>
          ))}
        </div>
        <div className="kb-grid-2">
          <Field label="Time windows (comma-separated HH:MM-HH:MM)"><input className="kb-input" value={slots} onChange={(e) => setSlots(e.target.value)} placeholder="10:00-13:00, 16:00-19:00" /></Field>
          <Field label="Daily capacity (kg, optional)"><input className="kb-input" value={capacityKg} onChange={(e) => setCapacityKg(e.target.value)} inputMode="numeric" placeholder="500" /></Field>
        </div>

        <h3 className="kb-card-title">Minimums</h3>
        <div className="kb-grid-2">
          <Field label="Min order value (₹, optional)"><input className="kb-input" value={minOrder} onChange={(e) => setMinOrder(e.target.value)} inputMode="numeric" placeholder="200" /></Field>
          <Field label="Min weight (kg, optional)"><input className="kb-input" value={minWeightKg} onChange={(e) => setMinWeightKg(e.target.value)} inputMode="decimal" placeholder="5" /></Field>
        </div>

        <h3 className="kb-card-title">Priced lines</h3>
        {lines.map((l, i) => (
          <div key={i} className="kb-card" style={{ marginBottom: 8 }}>
            <div className="kb-grid-2">
              <Field label="Category id"><input className="kb-input" value={l.category_id} onChange={(e) => setLine(i, { category_id: e.target.value })} placeholder="metal.copper" /></Field>
              <Field label="Grade (optional)"><input className="kb-input" value={l.grade} onChange={(e) => setLine(i, { grade: e.target.value })} placeholder="bright" /></Field>
            </div>
            <div className="kb-grid-2">
              <Field label="Unit">
                <select className="kb-input" value={l.unit} onChange={(e) => setLine(i, { unit: e.target.value as 'KG' | 'PIECE' })}>
                  <option value="KG">per kg</option>
                  <option value="PIECE">per piece</option>
                </select>
              </Field>
              <Field label={`Price ₹ ${l.unit === 'KG' ? 'per kg' : 'per piece'}`}><input className="kb-input" value={l.price} onChange={(e) => setLine(i, { price: e.target.value })} inputMode="decimal" placeholder="650" /></Field>
            </div>
            <Field label="Min weight for this line (kg, optional)"><input className="kb-input" value={l.min_weight_kg} onChange={(e) => setLine(i, { min_weight_kg: e.target.value })} inputMode="decimal" placeholder="5" style={{ maxWidth: 160 }} /></Field>
            {lines.length > 1 ? <Button variant="ghost" onClick={() => removeLine(i)}>Remove line</Button> : null}
          </div>
        ))}
        <Button variant="ghost" onClick={addLine}>+ Add line</Button>

        <h3 className="kb-card-title">Exclusions</h3>
        <Field label="Excluded category globs (comma-separated)"><input className="kb-input" value={exclusions} onChange={(e) => setExclusions(e.target.value)} placeholder="glass.*" /></Field>

        {saveForm.error ? <ErrorNote>{saveForm.error instanceof Error ? saveForm.error.message : 'Could not save the draft.'}</ErrorNote> : null}
        {saveForm.isSuccess ? <p className="kb-muted">Draft saved. Review, simulate and publish it above.</p> : null}
        <div style={{ marginTop: 12 }}>
          <Button disabled={saveForm.isPending} onClick={() => saveForm.mutate()}>{saveForm.isPending ? 'Saving…' : 'Save draft'}</Button>
        </div>
      </Card>

      {/* Published history */}
      <Card title="Published versions">
        {published.length === 0 ? <p className="kb-muted">Nothing published yet.</p> : (
          <ul className="kb-list">
            {published.map((rs) => (
              <li key={rs.version} className="kb-between">
                <span>v{rs.version} · {rs.lines.length} lines</span>
                <Badge tone="good">live</Badge>
              </li>
            ))}
          </ul>
        )}
      </Card>
    </>
  );
}
