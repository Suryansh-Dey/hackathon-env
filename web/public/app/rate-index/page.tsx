'use client';
import { Card, Spinner, ErrorNote, Field, Badge, formatPaise, formatRange } from '@kabados/ui';
import { useEffect, useState } from 'react';
import { FALLBACK_CITIES, cityLabel, fetchJson, queryParam, type ClientConfig, type RateIndexEntry } from '../lib';

export default function RateIndexPage() {
  const [mounted, setMounted] = useState(false);
  const [cities, setCities] = useState<string[]>(FALLBACK_CITIES);
  const [city, setCity] = useState('delhi');

  const [rows, setRows] = useState<RateIndexEntry[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Pick up ?city= and load the configured city list once mounted.
  useEffect(() => {
    setMounted(true);
    const q = queryParam('city').toLowerCase();
    if (q) setCity(q);
    const ac = new AbortController();
    fetchJson<ClientConfig>('/v1/public/config', ac.signal)
      .then((cfg) => {
        if (cfg.cities && cfg.cities.length) {
          setCities(cfg.cities);
          if (!q && !cfg.cities.includes('delhi')) setCity(cfg.cities[0]);
        }
      })
      .catch(() => {
        /* keep the fallback list */
      });
    return () => ac.abort();
  }, []);

  // Load the index whenever the city changes.
  useEffect(() => {
    if (!mounted || !city) return;
    const ac = new AbortController();
    setLoading(true);
    setError(null);
    fetchJson<RateIndexEntry[]>(`/v1/public/rate-index?city=${encodeURIComponent(city)}`, ac.signal)
      .then((data) => setRows(data))
      .catch((e: unknown) => {
        if ((e as { name?: string }).name !== 'AbortError') {
          setError(e instanceof Error ? e.message : 'Could not load the rate index.');
          setRows(null);
        }
      })
      .finally(() => setLoading(false));
    return () => ac.abort();
  }, [mounted, city]);

  if (!mounted) return <Spinner />;

  return (
    <>
      <h1 className="kb-h1">Kabad Rate Index</h1>
      <p className="kb-muted">
        Weekly median prices paid for scrap, computed from completed deals across verified vendors. A row is
        published only once enough independent deals and vendors contribute that week, so no single vendor or
        seller can be identified.
      </p>

      <Card>
        <Field label="City">
          <select className="kb-input" value={city} onChange={(e) => setCity(e.target.value)}>
            {cities.map((c) => (
              <option key={c} value={c}>
                {cityLabel(c)}
              </option>
            ))}
          </select>
        </Field>
      </Card>

      {loading ? <Spinner label="Loading rates…" /> : null}
      {error ? <ErrorNote>{error}</ErrorNote> : null}

      {!loading && !error && rows && rows.length === 0 ? (
        <p className="kb-empty">
          No published rates for {cityLabel(city)} yet. Rates appear once enough deals complete this week.
        </p>
      ) : null}

      {!loading && !error && rows && rows.length > 0 ? (
        <ul className="kb-list">
          {rows.map((r, i) => (
            <li key={`${r.category_id}-${r.grade}-${r.iso_week}-${i}`}>
              <div className="kb-between">
                <span>
                  <strong>{r.category_id}</strong>
                  {r.grade ? <> · {r.grade}</> : null}
                  <br />
                  <span className="kb-muted">
                    {r.iso_week} · {r.deals} {r.deals === 1 ? 'deal' : 'deals'} · {r.vendors}{' '}
                    {r.vendors === 1 ? 'vendor' : 'vendors'}
                  </span>
                </span>
                <span style={{ textAlign: 'right' }}>
                  <span className="kb-price">{formatPaise(r.median_paise_per_kg)}/kg</span>
                  <br />
                  <span className="kb-muted">
                    typical {formatRange(r.p25_paise_per_kg, r.p75_paise_per_kg)}
                  </span>
                </span>
              </div>
            </li>
          ))}
        </ul>
      ) : null}

      {!loading && !error && rows && rows.length > 0 ? (
        <p className="kb-muted">
          <Badge tone="good">Published</Badge> Median is the middle price per kg; the typical range is the 25th
          to 75th percentile of deals that week.
        </p>
      ) : null}
    </>
  );
}
