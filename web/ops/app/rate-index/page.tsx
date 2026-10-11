'use client';
import { Badge, Button, Card, ErrorNote, Field, Spinner, formatPaise } from '@kabados/ui';
import { useMutation } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import { StaffGate, msg, useOpsApi, type RateIndexEntry } from '../ops-api';

function currentIsoWeek(): string {
  const d = new Date();
  const target = new Date(Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()));
  const dayNr = (target.getUTCDay() + 6) % 7;
  target.setUTCDate(target.getUTCDate() - dayNr + 3);
  const firstThursday = new Date(Date.UTC(target.getUTCFullYear(), 0, 4));
  const week = 1 + Math.round(((target.getTime() - firstThursday.getTime()) / 86400000 - 3 + ((firstThursday.getUTCDay() + 6) % 7)) / 7);
  return `${target.getUTCFullYear()}-W${String(week).padStart(2, '0')}`;
}

export default function RateIndexPage() {
  return (
    <StaffGate>
      <RateIndexTool />
    </StaffGate>
  );
}

function RateIndexTool() {
  const api = useOpsApi();
  const [city, setCity] = useState('');
  const [isoWeek, setIsoWeek] = useState('');
  useEffect(() => setIsoWeek((w) => w || currentIsoWeek()), []);

  const publish = useMutation({
    mutationFn: async () => {
      const { data, error } = await api.POST('/v1/ops/rate-index:publish', { body: { city, iso_week: isoWeek } });
      if (error) throw error;
      return data as RateIndexEntry[];
    },
  });

  return (
    <>
      <h1 className="kb-h1">Rate index</h1>
      <Card title="Publish weekly index">
        <p className="kb-muted">Publishes the agreed-rate percentiles for a city and ISO week.</p>
        <div className="kb-grid kb-grid-2">
          <Field label="City">
            <input className="kb-input" value={city} onChange={(e) => setCity(e.target.value)} placeholder="e.g. Bengaluru" />
          </Field>
          <Field label="ISO week">
            <input className="kb-input" value={isoWeek} onChange={(e) => setIsoWeek(e.target.value)} placeholder="2026-W41" />
          </Field>
        </div>
        {publish.error ? <ErrorNote>{msg(publish.error)}</ErrorNote> : null}
        <Button onClick={() => publish.mutate()} disabled={publish.isPending || !city || !isoWeek}>
          {publish.isPending ? 'Publishing…' : 'Publish index'}
        </Button>
      </Card>

      {publish.isPending ? <Spinner /> : null}
      {publish.data ? (
        publish.data.length === 0 ? (
          <p className="kb-empty">No rows published — no settled deals for that city and week.</p>
        ) : (
          <Card title={`Published ${publish.data.length} rows`}>
            <table className="kb-list" style={{ width: '100%' }}>
              <thead>
                <tr><th>Category</th><th>Grade</th><th>P25 /kg</th><th>Median /kg</th><th>P75 /kg</th><th>Deals</th><th>Vendors</th><th></th></tr>
              </thead>
              <tbody>
                {publish.data.map((r, i) => (
                  <tr key={`${r.category_id}-${r.grade}-${i}`}>
                    <td>{r.category_id}</td>
                    <td>{r.grade || '—'}</td>
                    <td>{formatPaise(r.p25_paise_per_kg)}</td>
                    <td><strong>{formatPaise(r.median_paise_per_kg)}</strong></td>
                    <td>{formatPaise(r.p75_paise_per_kg)}</td>
                    <td>{r.deals}</td>
                    <td>{r.vendors}</td>
                    <td>{r.published ? <Badge tone="good">published</Badge> : <Badge tone="warn">draft</Badge>}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </Card>
        )
      ) : null}
    </>
  );
}
