'use client';
import { Card, Button } from '@kabados/ui';

export default function Home() {
  return (
    <>
      <h1 className="kb-h1">KabadOS</h1>
      <p className="kb-muted">
        KabadOS is a verified scrap marketplace for India. Sellers list their scrap, get an AI-checked estimate,
        and accept real offers from nearby verified vendors — with transparent prices and a signed receipt for
        every pickup. These public pages are open to everyone, no account needed.
      </p>

      <div className="kb-grid kb-grid-2">
        <Card title="Kabad Rate Index">
          <p className="kb-muted">
            See the weekly median price per kg paid for scrap in your city, built from completed deals across
            verified vendors.
          </p>
          <a href="/rate-index/">
            <Button>View rates</Button>
          </a>
        </Card>

        <Card title="City recycling impact">
          <p className="kb-muted">
            How much scrap was diverted from landfill, hazardous e-waste routed to authorized recyclers, and
            CO2e avoided in your city.
          </p>
          <a href="/impact/">
            <Button>See impact</Button>
          </a>
        </Card>

        <Card title="Verify a Green Receipt">
          <p className="kb-muted">
            Confirm that a receipt from a KabadOS pickup is a genuine, signed platform record.
          </p>
          <a href="/verify/">
            <Button>Verify a receipt</Button>
          </a>
        </Card>
      </div>
    </>
  );
}
