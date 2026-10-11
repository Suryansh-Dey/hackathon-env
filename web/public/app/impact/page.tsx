'use client';
import { Card, Spinner, ErrorNote, Field, Badge, formatWeight, formatDateTime } from '@kabados/ui';
import { useEffect, useState } from 'react';
import { FALLBACK_CITIES, cityLabel, fetchJson, queryParam, type ClientConfig, type CityImpact } from '../lib';

export default function ImpactPage() {
  const [mounted, setMounted] = useState(false);
  const [cities, setCities] = useState<string[]>(FALLBACK_CITIES);
  const [city, setCity] = useState('delhi');

  const [impact, setImpact] = useState<CityImpact | null>(null);
  const [hasData, setHasData] = useState(true);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

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

  useEffect(() => {
    if (!mounted || !city) return;
    const ac = new AbortController();
    setLoading(true);
    setError(null);
    fetchJson<CityImpact | null>(`/v1/public/impact?city=${encodeURIComponent(city)}`, ac.signal)
      .then((data) => {
        setImpact(data);
        setHasData(data != null);
      })
      .catch((e: unknown) => {
        if ((e as { name?: string }).name !== 'AbortError') {
          setError(e instanceof Error ? e.message : 'Could not load impact data.');
          setImpact(null);
        }
      })
      .finally(() => setLoading(false));
    return () => ac.abort();
  }, [mounted, city]);

  if (!mounted) return <Spinner />;

  return (
    <>
      <h1 className="kb-h1">City recycling impact</h1>
      <p className="kb-muted">
        What KabadOS deals diverted from landfill in this city: how much scrap was recycled, how much hazardous
        e-waste reached authorized recyclers, and an estimate of the CO2e avoided.
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

      {loading ? <Spinner label="Loading impact…" /> : null}
      {error ? <ErrorNote>{error}</ErrorNote> : null}

      {!loading && !error && !hasData ? (
        <p className="kb-empty">
          No impact recorded for {cityLabel(city)} yet. Numbers appear once deals complete in this city.
        </p>
      ) : null}

      {!loading && !error && impact ? (
        <>
          <div className="kb-grid kb-grid-2">
            <Card title="Hazardous routed safely">
              <p className="kb-price">{formatWeight(impact.hazardous_to_authorized)}</p>
              <p className="kb-muted">e-waste and batteries sent to authorized recyclers.</p>
            </Card>
            <Card title="CO2e avoided (estimate)">
              <p className="kb-price">{formatWeight(impact.co2e_avoided_g)}</p>
              <p className="kb-muted">
                Estimated, not measured — see the method below. <Badge tone="warn">estimate</Badge>
              </p>
            </Card>
          </div>

          <Card title="Diverted by category">
            {impact.kg_by_category.length === 0 ? (
              <p className="kb-empty">No category breakdown yet.</p>
            ) : (
              <ul className="kb-list">
                {impact.kg_by_category.map(([category, grams]) => (
                  <li key={category}>
                    <div className="kb-between">
                      <span>{category}</span>
                      <span className="kb-price">{formatWeight(grams)}</span>
                    </div>
                  </li>
                ))}
              </ul>
            )}
          </Card>

          <p className="kb-muted">
            Based on {impact.deals} completed {impact.deals === 1 ? 'deal' : 'deals'} · updated{' '}
            {formatDateTime(impact.updated_at)}.
          </p>
          <p className="kb-muted">
            Method: CO2e avoided is an estimate using emission factors from{' '}
            <strong>{impact.emission_factor_source}</strong> applied to the weight recycled per category. Treat
            it as an indicative figure, not a certified carbon credit.
          </p>
        </>
      ) : null}
    </>
  );
}
