# KabadOS demo guide

The judges don't see a live demo. They watch a **three-minute recorded video** that
shows "what it does, who it is for, and where AWS fits". This guide covers how to
prepare the deployed app, a shot-by-shot script for the recording, and talking points
mapped to each judging criterion.

**The story in one line:** a household photographs its scrap. AI itemises and checks it,
real vendors quote in seconds from their own rate cards, hazardous e-waste can only go
to authorised recyclers, and every rupee is settled from a scale reading, with a Green
Receipt anyone can verify.

---

## 1. What you need

| | URL / value |
|---|---|
| Seller site | https://d1kn7g0l0y77vz.cloudfront.net |
| Vendor site | https://dh36dlei05x8v.cloudfront.net |
| Ops console | https://d271ufmc8uwk5y.cloudfront.net |
| Public site (impact, receipt check) | https://d18zzn1na86duz.cloudfront.net |
| API | https://jl7jk1v3af.execute-api.ap-south-1.amazonaws.com |
| Seller phone | any fresh number, e.g. `+919812345601` (sign-in is phone-only, no OTP) |
| Authorised e-waste vendor | `+919800000002`, "GreenCycle E-Waste (authorized)", Delhi |
| Unauthorised vendor (for contrast) | `+919800000001`, "Raju Kabadiwala", Delhi |
| Other Delhi vendor | `+919800000003`, "Metro Metals" |
| Razorpay test payment | card `4111 1111 1111 1111`, any future expiry, any CVV, or UPI `success@razorpay` |

**Photos to bring:** 3–4 clear, well-lit phone photos of real scrap. The story works best with:
1. an old laptop: hazardous e-waste, which drives the authorised-recycler moment
2. a bundle of copper wire
3. a stack of newspapers
4. one wide shot with all of them together

---

## 2. Prepare (about 20 minutes, the day before)

### 2.1 Load the demo vendors

The seed step is safe to re-run: it only adds vendors that don't exist yet.

```bash
API=https://jl7jk1v3af.execute-api.ap-south-1.amazonaws.com
KEY=$(aws secretsmanager get-secret-value --region ap-south-1 \
  --secret-id /kabados/dev/admin_key --query SecretString --output text)
curl -s -XPOST $API/dev/seed -H "x-admin-key: $KEY" | jq '.vendors[].vendor'
```

The admin key is also in the git-ignored `.env` as `KABADOS_ADMIN_KEY`. Ops sign-in needs it.

### 2.2 Browser layout

- Use **two browser windows side by side**: seller on the left, vendor on the right.
  Each site keeps its own sign-in, so one browser is enough.
- Zoom to 110–125% so text is readable in the video.
- **Pages don't refresh themselves.** Press F5 on the other window after each action, and
  cut the reloads out in editing. The only exception is the AI-check step on the seller's
  "Sell your scrap" page, which updates every 2 seconds on its own.

### 2.3 Rehearse once, end to end

Run the flow in §3 once off camera. Platform payment through the Razorpay window is the
one step that hasn't been tested in a browser yet, so do it at least once before
recording. If it misbehaves, use the "pays directly" path described in §5.

### 2.4 Make a ready-made listing ("cooking show")

The AI check takes about **25–35 s**, and occasionally up to about 60 s. Make one listing
ahead of time and stop it at the "4. Check your items" step with the AI results showing.
On camera, start a fresh one, then cut to the ready one.

### 2.5 Rules that keep the demo working

- **Pickup pin:** in "1. Pickup address", type the pin `28.6139, 77.2090` and the city
  `Delhi`. Don't use "📍 Use my current location": vendors only serve 12 km around
  their Delhi and Bengaluru yards. A blank city can be geocoded as "new delhi", which
  doesn't count towards the Delhi impact page.
- **Run the AI before typing any items.** If items are already entered, the AI's results
  are not merged in. Run it once per listing, because each run adds claims again.
- **Add the seller's UPI ID first.** Go to Account → "Get paid by UPI" and enter e.g.
  `success@razorpay`. Accepting a "pays via platform" offer needs it.
- **Don't edit items after offers arrive.** That creates a new listing version, and the
  older offers can no longer be accepted.
- **Don't reload the vendor deal page while the Razorpay window is open.** If you dismiss
  the window, click the pay button again without reloading.
- **For the first 25 minutes after publishing, only matched vendors can offer.** This
  "first-look" window is a feature worth mentioning; the authorised vendor is matched
  within seconds.

---

## 3. The three-minute script

Each block below gives the timing, the shot, what to show on screen (exact labels), and
the voice-over (VO).

**0:00–0:15 · The problem.** Title card, then a photo of a real kabadiwala or a scrap heap.
> VO: "Indian households sell scrap blind. They don't know what it's worth and get low,
> opaque offers. E-waste and batteries end up with informal operators who process them
> unsafely, even though India has 472 authorised recyclers. KabadOS fixes that."

**0:15–0:50 · Seller: photos to verified items (seller window).**
1. Click "Sell scrap", fill "1. Pickup address", then "2. Describe your lot" ("old laptop,
   copper wire, newspapers") and "Start listing".
2. "Add photos" → "Upload from gallery" and pick the photos.
3. Click "Run AI analysis". The badge goes "queued" → "done" (cut to the ready-made listing).
4. In "4. Check your items", the rows fill in by themselves:
   - "AI found · 9x%"
   - the red "Hazardous · ewaste general" badge on the laptop
   - "⚠ Keep the battery inside; do not puncture or crush."
   - verification-level badges such as "Photo-checked"
   - a "Typical weight" hint with a one-click "use X"
5. "5. Suggested price" shows "₹X – ₹Y", based on N nearby vendors.

> VO: "Gemini itemises the photos and checks the seller's claims. Faces are blurred with
> Amazon Rekognition before any vendor sees a photo. But AI only informs: every price
> comes from vendors' own rate cards through one deterministic pricing engine."

**0:50–1:20 · Publish, matching and hazardous routing (vendor window).**
1. Seller: "6. Publish" → "Publish listing".
2. Vendor window, signed in as **GreenCycle**: "Matches" → "First-look leads". The new lot
   appears within seconds with "Nm first-look left".
3. Open it. The "Your rate-card quote" card shows the range, "% of items covered · km ·
   rate card vN" and explanation codes such as "verified bonus applied".
4. "Make an offer" → "Send offer".
5. Quick cut to **Raju** (unauthorised): the lot isn't in his feed at all, and the direct
   link shows "listing not found".

> VO: "Publishing fires an event through DynamoDB Streams and EventBridge, and matching
> runs on Lambda in seconds. Because the lot has a laptop, a Cedar forbid policy hides it
> from every vendor without a valid hazardous-waste authorisation. That rule isn't in the
> UI; it's enforced on every request."

**1:20–1:35 · Compare and accept (seller window).**
1. The listing page's "Offers" card shows the offer, ★ rating and "pays via platform".
2. Click "Accept". It goes straight to the deal page.

**1:35–2:15 · Pickup: weigh, pay, hand over (both windows).**
1. Vendor: "Mark arrived at pickup", then type the reading per line into "grams on scale"
   and click "Record weight". The "Final weighed amount" is calculated.
2. Seller: "Confirm weights" ("Confirm to lock the final amount").
3. Vendor: "Pay ₹… with Razorpay" → Razorpay test window → `success@razorpay`.
4. Seller: the big **4-digit "Handover code"** appears.
5. Vendor: "Handover OTP (from seller)" → "Confirm handover", then "Confirm delivery at yard".

> VO: "The final amount is the same pricing engine applied to the scale reading, so the
> quote and the settlement can't drift apart. Money is locked once the seller confirms,
> the vendor pays through Razorpay, and the goods only leave once the seller is paid.
> The code proves the handover. Every movement is a balanced double-entry ledger posting."

**2:15–2:40 · Proof and impact.**
1. Seller: the "♻ Green Receipt" card shows lines, weights, the "Hazardous" badge and
   "CO₂e avoided". Click "Open verify page".
2. Public site: "Verified Green Receipt" with "Verified" and "Authorized for hazardous".
3. Change one character of `sig` in the URL: "Could not verify this receipt."
4. Public "City recycling impact" for Delhi shows "Hazardous routed safely" and
   "Diverted by category", updated seconds after the deal settled.

> VO: "Anyone — a housing society, a CSR auditor — can verify the receipt; it's signed
> server-side and tamper-evident. Settling also updates the city's impact numbers, which
> are measured from scale readings, not estimates."

**2:40–3:00 · Architecture and close.** Architecture slide (see §4).
> VO: "Rust on AWS Lambda, DynamoDB with Streams, EventBridge and SQS, S3 and
> CloudFront, KMS field encryption, Secrets Manager, Rekognition and Amazon Location,
> with GitHub Actions CI. KabadOS: fair prices for households, records for kabadiwalas,
> and hazardous waste only to authorised recyclers."

---

## 4. Where AWS fits (be precise)

**Live in the deployed app (safe to claim):**

| Service | What it does in KabadOS |
|---|---|
| AWS Lambda (Rust, arm64) | One API function (axum) plus 4 workers: stream/outbox, AI analysis, matching events, impact/Rate Index |
| API Gateway (HTTP API) | Public API; CORS restricted to the four sites |
| DynamoDB | Single-table design (core, with 3 GSIs), plus ledger, chat, ephemeral (TTL) and audit tables; point-in-time recovery on |
| DynamoDB Streams + EventBridge | Transactional outbox → domain events → matching and impact workers |
| SQS (+ DLQs) | Photo processing and AI analysis queues |
| S3 + CloudFront | Photos via presigned uploads; four static sites; a CloudFront Function for routing |
| Amazon Rekognition | Face detection for blurring, and moderation on every listing photo |
| Amazon Location Service | Road distances for matching and offer quotes; reverse geocoding of addresses |
| AWS KMS | Envelope encryption of phone numbers, addresses and handover codes |
| AWS Secrets Manager | Gemini key, Razorpay test keys, pepper, admin key |
| EventBridge (cron rule) | Weekly Rate Index computation |
| GitHub Actions + OIDC | CI on every push (Rust, infra, web, transporter app) and a deploy job for dev |

**Also used:**
- Google Gemini via `gemini-client-api` (itemise, verify, scale reading, rate-card drafting).
- Cedar policies (56 files, in-process).
- Razorpay in **test mode**.

**Don't claim these as live.** They're in the plan or in code but not switched on in the
deployed app:
- Amazon Cognito: sign-in is phone-only with signed tokens.
- Amazon Verified Permissions: the same Cedar policies run in-process instead.
- Step Functions.
- AppSync realtime.
- SNS push and SES email.
- Textract.
- Chime calls: there's an API but no UI.
- WhatsApp/SMS, KYC, Razorpay Route/Payouts and Play Integrity are mocked by design for
  the hackathon.

---

## 5. Backup plans

- **Slow AI:** cut to the ready-made listing (§2.4).
- **Razorpay window misbehaves:** on the vendor offer, choose "Direct to seller (Mode B)".
  The flow becomes: vendor "Mark paid directly (Mode B)" → seller "I received payment" →
  handover code → same ending.
- **No lead under "Matches":** reload after a few seconds; matching runs asynchronously.
  Make sure the pin is `28.6139, 77.2090`.
- **"this listing is in its first-look window":** you're signed in as a vendor that
  wasn't matched. For the laptop lot, use GreenCycle (`+919800000002`).

---

## 6. Talking points per judging criterion

- **01 Idea and Impact:**
  - A real, local problem: unsafe e-waste handling and opaque scrap prices.
  - What changes: households get fair, comparable offers. Kabadiwalas get digital demand
    and records. Hazardous items go only to authorised recyclers, enforced by policy.
  - Impact is measured from scale readings: kg diverted per city and category, shown
    live on the public Impact page.
- **02 Built on AWS:** fully deployed on AWS (§4), serverless and event-driven; the
  backend is Rust on Lambda.
- **03 Design and usability:**
  - Phone-number sign-in and a guided, numbered listing flow ("1. Pickup address" …
    "6. Publish").
  - The AI fills in the item list and suggests weights.
  - A big 4-digit handover code, and plain-language errors.
- **04 The execution:** everything in the video runs on the deployed stack. CI checks
  every push, with property-based tests on the pricing engine and Cedar policy tables.
- **05 The demo video:** follow §3, and keep it at 3:00.

Lines worth quoting:
- "Deterministic code decides money and permissions; AI informs."
- "The same pricing function computes the suggestion, the offer and the settlement, so
  they can't drift apart."

---

## 7. Optional extras (for Q&A or a longer cut)

- **Vendor rate card by typing:**
  - "Rate card" → "Draft from text", e.g. "copper 650, brass 400, paper 14 above 5 kilo,
    no glass". Gemini drafts the card, but it only goes live after "Simulate" and "Publish".
  - Typing "lead acid battery 90" as Raju shows the validation issue that blocks
    publishing without an authorisation.
- **Hazardous authorisation:** vendor dashboard card "Hazardous-waste authorization".
- **Ops console:**
  - Sign in with any +91 number plus the admin key.
  - "Config" shows the provider modes.
  - "KYC" → status "approved" → "Reject" a transporter, and Cedar blocks them immediately.
- **Transporter app** (Dioxus desktop, uses the same AWS API):
  1. One-time setup: `sudo pacman -S xdotool` (provides `libxdo`).
  2. Start it: `cd apps/transporter && cargo run --release`.
  3. Driver: sign in with a fresh number, "Start driving", then "Go on duty".
  4. Vendor: "Transporters" → "Invite a transporter". Make the offer with "Use a
     transporter", then on the deal "Assign transporter", and set "Accept timeout" to 1800 s.
  5. The driver sees the offer within 5 s, then works through the checklist: weigh each
     item, wait for payment, then the "Verify code & load" handover.
- **Skip on camera:**
  - The Rate Index page stays empty until a city has 20 deals from 5 vendors in a week
    (that's the k-anonymity rule, and you can pitch it as such).
  - The ops "AI review" queue is empty.
  - CO₂e uses placeholder emission factors, so call it an estimate.
