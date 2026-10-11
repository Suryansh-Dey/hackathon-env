// Razorpay Checkout (standard web checkout). The script is injected once on
// demand so pages that never take a payment don't load it.

export type CheckoutSuccess = {
  razorpay_payment_id: string;
  razorpay_order_id: string;
  razorpay_signature: string;
};

type RazorpayInstance = { open(): void; on(event: string, cb: (r: { error?: { description?: string } }) => void): void };

declare global {
  interface Window {
    Razorpay?: new (options: Record<string, unknown>) => RazorpayInstance;
  }
}

const SRC = 'https://checkout.razorpay.com/v1/checkout.js';
let loading: Promise<void> | null = null;

export function loadRazorpay(): Promise<void> {
  if (window.Razorpay) return Promise.resolve();
  if (!loading) {
    loading = new Promise<void>((resolve, reject) => {
      const s = document.createElement('script');
      s.src = SRC;
      s.async = true;
      s.onload = () => resolve();
      s.onerror = () => {
        loading = null;
        s.remove();
        reject(new Error('Could not load Razorpay Checkout. Check your connection and try again.'));
      };
      document.body.appendChild(s);
    });
  }
  return loading;
}

/**
 * Opens Checkout for an existing order. Resolves with the signed result on
 * success; rejects when the payer closes the modal (with the last failure
 * reason, if a payment attempt failed).
 */
export async function openCheckout(o: { key: string; orderId: string; amount: number; currency: string; description: string }): Promise<CheckoutSuccess> {
  await loadRazorpay();
  const Razorpay = window.Razorpay;
  if (!Razorpay) throw new Error('Razorpay Checkout is unavailable.');
  return new Promise<CheckoutSuccess>((resolve, reject) => {
    let lastFailure: string | null = null;
    const rzp = new Razorpay({
      key: o.key,
      order_id: o.orderId,
      amount: o.amount,
      currency: o.currency,
      name: 'KabadOS',
      description: o.description,
      handler: (r: CheckoutSuccess) => resolve(r),
      modal: { ondismiss: () => reject(new Error(lastFailure ? `Payment failed: ${lastFailure}` : 'Payment was cancelled.')) },
    });
    rzp.on('payment.failed', (r) => {
      lastFailure = r.error?.description ?? 'payment failed';
    });
    rzp.open();
  });
}
