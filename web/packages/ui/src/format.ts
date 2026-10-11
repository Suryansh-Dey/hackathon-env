// en-IN money, weight and date formatting shared by every site (PLAN.md §15.5).

/** Paise (integer) → "₹1,23,456.78" with Indian digit grouping. */
export function formatPaise(paise: number): string {
  const neg = paise < 0;
  const abs = Math.abs(paise);
  const rupees = Math.floor(abs / 100);
  const p = abs % 100;
  const grouped = new Intl.NumberFormat('en-IN').format(rupees);
  const sign = neg ? '-' : '';
  return p === 0 ? `${sign}₹${grouped}` : `${sign}₹${grouped}.${p.toString().padStart(2, '0')}`;
}

/** A paise range → "₹100 – ₹150" (or a single value when equal). */
export function formatRange(low: number, high: number): string {
  return low === high ? formatPaise(low) : `${formatPaise(low)} – ${formatPaise(high)}`;
}

/** Grams (integer) → "42.6 kg" or "850 g". */
export function formatWeight(grams: number): string {
  if (grams < 1000) return `${grams} g`;
  const kg = grams / 1000;
  return `${new Intl.NumberFormat('en-IN', { maximumFractionDigits: 1 }).format(kg)} kg`;
}

export function formatDate(iso: string): string {
  return new Date(iso).toLocaleDateString('en-IN', { day: 'numeric', month: 'short', year: 'numeric' });
}

export function formatDateTime(iso: string): string {
  return new Date(iso).toLocaleString('en-IN', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' });
}

/** Stars out of 5 as "★★★★☆". */
export function stars(rating: number): string {
  const full = Math.round(rating);
  return '★'.repeat(full) + '☆'.repeat(Math.max(0, 5 - full));
}
