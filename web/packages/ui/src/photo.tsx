'use client';
// Guided photo capture + upload for the seller website (PLAN.md §6.1): images
// are resized in the browser and sent straight to storage with a presigned PUT,
// then marked complete so the backend processes them (EXIF strip, face blur,
// thumbnail, triage). Works against S3 (deployed) and the local dev blob store.
// The page shows the uploaded photos from the listing, so there are no local previews.
import { useState } from 'react';
import { useAuth } from './auth';
import { apiBaseUrl } from './query';

type Kind = 'WIDE' | 'CLOSE_UP' | 'PROOF' | 'SCALE';
type Source = 'GALLERY' | 'CAMERA';

async function resize(file: File, max = 1600, quality = 0.85): Promise<Blob> {
  try {
    const bmp = await createImageBitmap(file);
    const scale = Math.min(1, max / Math.max(bmp.width, bmp.height));
    const w = Math.max(1, Math.round(bmp.width * scale));
    const h = Math.max(1, Math.round(bmp.height * scale));
    const canvas = document.createElement('canvas');
    canvas.width = w;
    canvas.height = h;
    const ctx = canvas.getContext('2d');
    if (!ctx) return file;
    ctx.drawImage(bmp, 0, 0, w, h);
    const blob = await new Promise<Blob | null>((r) => canvas.toBlob(r, 'image/jpeg', quality));
    return blob ?? file;
  } catch {
    return file; // fall back to the original bytes if canvas is unavailable
  }
}

function key(prefix: string): string {
  const rnd = typeof crypto !== 'undefined' && 'randomUUID' in crypto ? crypto.randomUUID() : `${Date.now()}-${Math.random()}`;
  return `${prefix}-${rnd}`;
}

export interface PhotoUploadProps {
  listingId: string;
  kind?: Kind;
  source?: Source;
  proofRequestId?: string;
  label?: string;
  onUploaded?: (imageId: string) => void;
}

export function PhotoUpload({ listingId, kind = 'WIDE', source = 'GALLERY', proofRequestId, label, onUploaded }: PhotoUploadProps) {
  const { token } = useAuth();
  const base = apiBaseUrl();
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function handle(files: FileList | null) {
    if (!files || !listingId) return;
    setBusy(true);
    setErr(null);
    try {
      for (const file of Array.from(files)) {
        const blob = await resize(file);
        const presign = await fetch(`${base}/v1/listings/${listingId}/images:presign`, {
          method: 'POST',
          headers: { 'content-type': 'application/json', authorization: `Bearer ${token}`, 'idempotency-key': key('img') },
          body: JSON.stringify({ kind, source, content_type: 'image/jpeg', size: blob.size, proof_request_id: proofRequestId ?? null }),
        });
        if (!presign.ok) throw new Error((await presign.json().catch(() => ({})))?.detail ?? 'Could not start upload');
        const { image_id, upload } = (await presign.json()) as { image_id: string; upload: { url: string; method: string; headers: [string, string][] } };
        const headers: Record<string, string> = {};
        for (const [k, v] of upload.headers) if (k.toLowerCase() === 'content-type') headers[k] = v;
        const put = await fetch(upload.url, { method: upload.method, headers, body: blob });
        if (!put.ok) throw new Error('Upload failed');
        const complete = await fetch(`${base}/v1/listings/${listingId}/images/${image_id}:complete`, {
          method: 'POST',
          headers: { authorization: `Bearer ${token}`, 'idempotency-key': `imgc-${image_id}` },
        });
        if (!complete.ok) throw new Error((await complete.json().catch(() => ({})))?.detail ?? 'Could not finalize the upload');
        onUploaded?.(image_id);
      }
    } catch (e) {
      setErr(e instanceof Error ? e.message : 'Upload error');
    } finally {
      setBusy(false);
    }
  }

  return (
    <div>
      <label className="kb-btn kb-btn-ghost" style={{ display: 'inline-block' }}>
        {busy ? 'Uploading…' : (label ?? 'Add photos')}
        <input
          type="file"
          accept="image/*"
          capture={source === 'CAMERA' ? 'environment' : undefined}
          multiple={source !== 'CAMERA'}
          style={{ display: 'none' }}
          disabled={busy || !listingId}
          onChange={(e) => handle(e.target.files)}
        />
      </label>
      {err ? <p className="kb-error">{err}</p> : null}
    </div>
  );
}
