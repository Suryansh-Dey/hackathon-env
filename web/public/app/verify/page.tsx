'use client';
import { Card, Button, Spinner, ErrorNote, Field, Badge, formatPaise, formatWeight, formatDateTime } from '@kabados/ui';
import { useEffect, useState, type FormEvent } from 'react';
import { fetchJson, queryParam, type GreenReceipt } from '../lib';

export default function VerifyPage() {
  const [mounted, setMounted] = useState(false);
  const [id, setId] = useState('');
  const [sig, setSig] = useState('');

  const [receipt, setReceipt] = useState<GreenReceipt | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [checked, setChecked] = useState(false);

  async function verify(receiptId: string, signature: string) {
    if (!receiptId || !signature) return;
    setLoading(true);
    setError(null);
    setReceipt(null);
    try {
      const data = await fetchJson<GreenReceipt>(
        `/v1/public/receipts/${encodeURIComponent(receiptId)}?sig=${encodeURIComponent(signature)}`,
      );
      setReceipt(data);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : 'Could not verify this receipt.');
    } finally {
      setChecked(true);
      setLoading(false);
    }
  }

  // Auto-verify when the page is opened from a shared link (?id=&sig=).
  useEffect(() => {
    setMounted(true);
    const qid = queryParam('id');
    const qsig = queryParam('sig');
    setId(qid);
    setSig(qsig);
    if (qid && qsig) void verify(qid, qsig);
  }, []);

  function onSubmit(e: FormEvent) {
    e.preventDefault();
    // Reflect the inputs in the URL so the result stays shareable.
    if (typeof window !== 'undefined') {
      const url = `${window.location.pathname}?id=${encodeURIComponent(id)}&sig=${encodeURIComponent(sig)}`;
      window.history.replaceState(null, '', url);
    }
    void verify(id.trim(), sig.trim());
  }

  if (!mounted) return <Spinner />;

  return (
    <>
      <h1 className="kb-h1">Verify a Green Receipt</h1>
      <p className="kb-muted">
        Every completed KabadOS pickup issues a signed Green Receipt. Paste its ID and signature, or open a
        shared link, to confirm it is a genuine platform record.
      </p>

      <Card title="Check a receipt">
        <form onSubmit={onSubmit}>
          <Field label="Receipt ID">
            <input
              className="kb-input"
              value={id}
              onChange={(e) => setId(e.target.value)}
              placeholder="rcp_…"
              autoComplete="off"
            />
          </Field>
          <Field label="Signature">
            <input
              className="kb-input"
              value={sig}
              onChange={(e) => setSig(e.target.value)}
              placeholder="signature from the receipt link"
              autoComplete="off"
            />
          </Field>
          <Button type="submit" disabled={loading || !id.trim() || !sig.trim()}>
            {loading ? 'Verifying…' : 'Verify'}
          </Button>
        </form>
      </Card>

      {loading ? <Spinner label="Verifying…" /> : null}

      {!loading && error ? (
        <Card>
          <ErrorNote>Could not verify this receipt.</ErrorNote>
          <p className="kb-muted">
            The ID or signature may be wrong, or the receipt does not exist. {error}
          </p>
        </Card>
      ) : null}

      {!loading && !error && checked && receipt ? (
        <Card title="Verified Green Receipt">
          <p>
            <Badge tone="good">Verified</Badge>
          </p>
          <div className="kb-between">
            <span>
              <strong>{receipt.vendor_name}</strong>
              <br />
              <span className="kb-muted">
                {receipt.city} · issued {formatDateTime(receipt.issued_at)}
              </span>
            </span>
            <span>
              {receipt.vendor_authorized_for_hazardous ? (
                <Badge tone="good">Authorized for hazardous</Badge>
              ) : null}
            </span>
          </div>

          <ul className="kb-list">
            {receipt.lines.map((line, i) => (
              <li key={`${line.category_id}-${i}`}>
                <div className="kb-between">
                  <span>
                    {line.category_name}
                    {line.hazardous ? (
                      <>
                        {' '}
                        <Badge tone="hazard">hazardous</Badge>
                      </>
                    ) : null}
                  </span>
                  <span className="kb-price">{formatWeight(line.weight)}</span>
                </div>
              </li>
            ))}
          </ul>

          <div className="kb-between">
            <strong>Total weight</strong>
            <span className="kb-price">{formatWeight(receipt.total_weight)}</span>
          </div>
          <div className="kb-between">
            <strong>Total amount</strong>
            <span className="kb-price">{formatPaise(receipt.total_amount)}</span>
          </div>
          <div className="kb-between">
            <strong>CO2e avoided (estimate)</strong>
            <span className="kb-price">{formatWeight(receipt.co2e_avoided_g)}</span>
          </div>

          <p className="kb-muted">{receipt.disclaimer}</p>
          <p className="kb-muted">CO2e method: {receipt.emission_factor_source}.</p>
        </Card>
      ) : null}
    </>
  );
}
