import '@kabados/ui/styles.css';
import type { ReactNode } from 'react';
import { Header } from './header';
export const metadata = { title: 'KabadOS — Kabad Rate Index & impact', description: 'Open scrap price reference and recycling impact for India.' };
export default function RootLayout({ children }: { children: ReactNode }) {
  return (<html lang="en"><body><Header /><main className="kb-container">{children}</main></body></html>);
}
