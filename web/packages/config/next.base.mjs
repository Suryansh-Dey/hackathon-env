// Shared Next.js config: static export, no server runtime (PLAN.md §6).
/** @type {import('next').NextConfig} */
export const base = {
  output: 'export',
  reactStrictMode: true,
  images: { unoptimized: true },
  trailingSlash: true,
  // Per-resource pages use query params (?id=…) since export can't pre-render per-ID routes.
  env: { NEXT_PUBLIC_API_URL: process.env.NEXT_PUBLIC_API_URL ?? '' },
};
