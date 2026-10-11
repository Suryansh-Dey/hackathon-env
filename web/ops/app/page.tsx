'use client';
import { Card } from '@kabados/ui';

const SECTIONS: { href: string; title: string; desc: string }[] = [
  { href: '/kyc/', title: 'KYC review', desc: 'Approve or reject vendor and transporter documents.' },
  { href: '/disputes/', title: 'Disputes', desc: 'Review deal evidence and resolve open disputes.' },
  { href: '/ai-reviews/', title: 'AI review', desc: 'Flagged listings: low confidence, failed-closed, contested.' },
  { href: '/rate-index/', title: 'Rate index', desc: 'Publish the weekly agreed-rate percentiles.' },
  { href: '/config/', title: 'Config', desc: 'Platform settings and user suspension.' },
];

export default function Home() {
  return (
    <>
      <h1 className="kb-h1">Ops console</h1>
      <div className="kb-grid kb-grid-2">
        {SECTIONS.map((s) => (
          <a key={s.href} href={s.href} style={{ textDecoration: 'none', color: 'inherit' }}>
            <Card title={s.title}><p className="kb-muted">{s.desc}</p></Card>
          </a>
        ))}
      </div>
    </>
  );
}
