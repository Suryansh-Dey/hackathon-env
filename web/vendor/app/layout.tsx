import '@kabados/ui/styles.css';
import type { ReactNode } from 'react';
import { AppProviders } from './providers';
import { Header } from './header';
export const metadata = { title: 'KabadOS vendor' };
export default function RootLayout({ children }: { children: ReactNode }) {
  return (<html lang="en"><body><AppProviders><Header /><main className="kb-container">{children}</main></AppProviders></body></html>);
}
