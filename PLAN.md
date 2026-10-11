# KabadOS — Architecture & Technical Plan

> A verified scrap marketplace for India. Sellers list scrap with photos; an AI itemizes it, checks claims and asks for proof; vendors' own pricing rules produce instant quotes; vendors or their transporters pick up; every kilo is weighed, paid for and traced.

**Stack at a glance:** Rust backend on AWS Lambda · Next.js static-export websites (seller, vendor, ops, public) · Dioxus (Rust) Android app for transporters · AWS-managed services first for identity, messaging, maps, calls and data; Google Gemini API for AI.

External services and their setup are listed in [`external_api.md`](external_api.md).

---

## Table of contents

1. [Overview](#1-overview)
2. [Core features and rationale](#2-core-features-and-rationale)
3. [Roles](#3-roles)
4. [End-to-end flow and state machines](#4-end-to-end-flow-and-state-machines)
5. [Identity and sign-in (AWS-native)](#5-identity-and-sign-in-aws-native)
6. [Seller website](#6-seller-website)
7. [Vendor website](#7-vendor-website)
8. [Transporter app (Dioxus)](#8-transporter-app-dioxus)
9. [Pooled collection drives](#9-pooled-collection-drives)
10. [Ops console and public pages](#10-ops-console-and-public-pages)
11. [AI system](#11-ai-system)
12. [Pricing rules, quoting and matching](#12-pricing-rules-quoting-and-matching)
13. [Settlement, payments and ledger](#13-settlement-payments-and-ledger)
14. [Trust, safety and fraud](#14-trust-safety-and-fraud)
15. [System architecture and tech stack](#15-system-architecture-and-tech-stack)
16. [Data model (Amazon DynamoDB)](#16-data-model-amazon-dynamodb)
17. [API surface, events and real-time](#17-api-surface-events-and-real-time)
18. [Authorization with Cedar](#18-authorization-with-cedar)
19. [Notifications and calls](#19-notifications-and-calls)
20. [Security, privacy and compliance](#20-security-privacy-and-compliance)
21. [Infrastructure, environments and delivery](#21-infrastructure-environments-and-delivery)
22. [Observability, SLOs, metrics and testing](#22-observability-slos-metrics-and-testing)
23. [Glossary](#23-glossary)

---

## 1. Overview

### 1.1 Problem
- Households and small businesses hold recyclable scrap (paper, plastics, metals, appliances, e-waste) but don't know what it's worth and get low, opaque offers.
- The informal sector (waste pickers → kabadiwalas → aggregators) does most of India's recycling but works in cash, without records and without access to digital demand ([ITU on Kabadiwalla Connect](https://aiforgood.itu.int/indian-firms-digital-solution-for-urban-waste-pickers/)).
- E-waste and batteries often reach informal operators who process them unsafely, although India has 472 SPCB-authorized dismantlers/recyclers in 22 states (CPCB figure). Sellers have no easy way to route hazardous items to them.
- Every vendor re-asks the same questions about a listing, so small listings aren't worth their time and end up in landfill or burning piles.

### 1.2 Product
A marketplace where **listings are pre-verified, prices come from vendors' own rules, pickups are proven, and the environmental outcome is recorded**. All clients talk to one Rust API:
- **Seller website** (Next.js static export, PWA): list scrap, compare offers, track pickup, get paid.
- **Vendor website** (Next.js static export, PWA): maintain a rate card, receive matched listings, make offers, assign transporters.
- **Transporter app** (Dioxus, Rust, Android): receive job offers, complete the pickup checklist, get paid.
- **Ops console** and **public pages** (Next.js static export).

### 1.3 Goals
| Goal | Metric | Target |
|---|---|---|
| Sellers get fair, fast offers | Median time from publish to first offer | < 2 hours in served areas |
| Vendors waste less time on bad leads | Deals with no item mismatch at pickup | > 90% |
| Honest, auditable settlement | Deals settled from a weighed scale reading | 100% |
| Hazardous items reach authorized channels | Hazardous kg picked up by authorized vendors | 100% (enforced) |
| Material diverted from landfill | kg diverted per month, by category | Measured from scale readings |
| Vendors earn more and keep records | Vendor monthly active, repeat rate, ledger usage | Measured per city |

### 1.4 Scope boundaries
- The platform never takes title to material; it connects parties and records the transaction.
- The platform never holds customer funds in its own wallet (RBI-regulated); money moves through a payment aggregator (§13).
- The platform doesn't issue EPR certificates; only CPCB-registered recyclers can, under the E-Waste (Management) Rules 2022.
- Municipal wet and sanitary waste are out of scope.

### 1.5 Architectural principles
1. **AWS-managed services first.** Identity, messaging, maps, calls, document and image analysis, storage, eventing, CI/CD and observability use AWS services. Third parties: the Google Gemini API for AI (existing paid plan), and services AWS has no equivalent for (payments, Google Play distribution and integrity, Android push transport, government-record KYC checks).
2. **One backend language.** All server code (APIs, workers, AI agents, pricing) is Rust on Lambda; the transporter app shares Rust crates with it.
3. **No server runtime in the websites.** Next.js is used as a static site generator with client-side rendering; every dynamic read and write goes through the Rust API.
4. **Deterministic code decides money and permissions;** AI informs (§11.1).

---

## 2. Core features and rationale

| # | Feature | Why | Goal |
|---|---|---|---|
| 1 | **AI itemization** from photos | Sellers don't know what their scrap is called or worth. | A structured item list in seconds; sellers accept AI categories unchanged for ≥ 80% of items. |
| 2 | **Claim verification + proof requests + verification ladder** | Vendors otherwise repeat the same proof requests, and distrust claims like "pure copper". | Listings arrive pre-verified; item-mismatch at pickup < 10% overall, < 3% for L2+ items. |
| 3 | **Suggested price with seller override** | Sellers need an anchor based on real market data but stay in control. | Every listing shows a suggested range from live vendor quotes; sellers can set an asking price with a reason. |
| 4 | **Vendor rate cards → deterministic quotes** | Vendors can't price hundreds of small listings by hand; prices must be auditable. | Every listing gets real quotes from real vendors' prices within seconds, 24×7. |
| 5 | **Plain-text rate card editing** | Filling a long form for every price change is slow for busy vendors. | Vendors type a sentence ("copper 650, paper 14 above 5 kilo") and get a structured draft to confirm. |
| 6 | **Agentic matching with caching** | Mixed lots, sparse areas and "what would get me a better offer?" need multi-step reasoning. | High match rate at low cost; most matches served from cache. |
| 7 | **Structured offers + chat** | Most negotiations end in a rate per kg; structured offers are comparable and fast. | Sellers compare offers side by side; chat only for edge cases. |
| 8 | **Scale-photo settlement + handover OTP** | Weight disputes and fake pickups are the main trust failures in scrap trading. | Every rupee traces to a scale reading, a seller confirmation and a GPS/time stamp. |
| 9 | **Vendor-chosen transporters** | Vendors have repeat, trusted drivers; sellers transact too rarely to build that trust. | Median time from deal to transporter acceptance < 15 min. |
| 10 | **Hazardous routing (Cedar-enforced)** | E-waste and batteries harm people and soil when processed informally. | 100% of hazardous lots go only to authorized vendors, enforced in the authorization layer. |
| 11 | **Pooled collection drives** | One household's scrap is often too small to justify a pickup. | Societies aggregate scrap into one pickup; pooled listings get offers ≥ 3× more often than solo listings under 10 kg. |
| 12 | **Kabad Rate Index** | No open, trustworthy scrap price reference exists for households. | Public weekly ₹/kg medians per city and category from completed deals, anonymized. |
| 13 | **Ground-truth data loop** | AI estimates need correction from reality. | Every weighing labels its photos (actual category, grade, weight) and feeds AI evaluation. |
| 14 | **Green Receipts + chain of custody** | Societies, offices and bulk waste generators need records of where their waste went. | Downloadable, verifiable receipts per pickup; aggregated impact reports. |
| 15 | **Honesty and reliability scores** | Good actors should be rewarded and bad actors visible. | Seller weight-honesty, vendor on-time and payment-speed, transporter acceptance and on-time scores. |
| 16 | **In-app calls, no phone numbers exchanged** | Sharing personal numbers is a privacy and safety risk and invites off-platform deals. | All seller/vendor/transporter calls run inside the apps over Amazon Chime SDK. |
| 17 | **Privacy by design** | Home addresses, faces and phone numbers appear in a scrap marketplace. | Progressive disclosure of address, face blurring, no number sharing; DPDP-compliant. |

---

## 3. Roles

| Role | Who | Client | Primary job |
|---|---|---|---|
| **Seller** | Household, shop, office, society resident | Seller website (phone browser or desktop) | Get a fair price and a hassle-free pickup. |
| **Vendor** | Kabadiwala, scrap dealer, aggregator, authorized e-waste dismantler/recycler | Vendor website (Android phone browser or desktop) | Buy material that fits their trade at a margin, with minimal wasted trips. |
| **Transporter** | Tempo, e-rickshaw or truck owner-driver | Transporter app (Android) | Get jobs, reach the right place, get paid. |
| **Society admin** | RWA secretary, office facilities manager | Seller website (drive admin view) | Run collection drives; get impact reports. |
| **Ops agent** | KabadOS staff | Ops console | Verify KYC, review AI edge cases, resolve disputes. |
| **Public** | Anyone | Public pages | View the Rate Index, city impact and receipt verification. |

One account can hold several roles (e.g., a vendor who also sells). Roles are attributes of a single Cognito user, not separate accounts.

---

## 4. End-to-end flow and state machines

### 4.1 Main flow (transporter pickup, platform-mediated payment)
```
Seller                    Platform (AI + engines)                 Vendor                 Transporter
  │ photos + weight + text ─►│ itemize (Gemini vision)               │                        │
  │                          │ verify claims → proof requests        │                        │
  │◄─ "close-up of coil?" ───│                                       │                        │
  │ extra photo ────────────►│ ladder ↑, suggested price from quotes │                        │
  │ publish (ask ₹ or accept suggestion)                             │                        │
  │                          │ match: coverage index → rules engine → rank → cache            │
  │                          │ notify top-K (first-look window) ────►│                        │
  │                          │                                       │ offer ₹/kg + slot      │
  │◄─ compare offers ────────│◄──────────────────────────────────────│                        │
  │ accept ─────────────────►│ deal created; exact address released to vendor (Cedar)         │
  │                          │                                       │ pick transporter ─────►│ job offer (WebSocket / FCM)
  │                          │                                       │                        │ accept → navigate
  │                          │◄──────────── arrive (Location Service geofence) ──────────────│
  │                          │◄──────────── photos + scale photo ────────────────────────────│
  │◄─ "42.6 kg, ₹ total" ────│ final = rate × weighed kg (same pricing crate)                 │
  │ confirm weight ─────────►│ request payment ─────────────────────►│ pay (UPI)              │
  │◄─ "₹ paid" ──────────────│◄─ payment captured (webhook) ─────────│                        │
  │ give handover OTP ─────────────────────────────────────────────────────────────────────►│ load
  │                          │◄──────────── delivered at yard (geofence + vendor confirm) ───│
  │◄─ Green Receipt ─────────│ ledger, impact, ratings, Rate Index, AI ground-truth labels     │
```

### 4.2 Listing states
```
DRAFT → ANALYZING → NEEDS_INPUT? → READY → PUBLISHED → RESERVED → COMPLETED
                         │                     │           │
                         └── seller skips ─────┘           ├→ PARTIALLY_COMPLETED (some lines rejected at pickup)
PUBLISHED → EXPIRED (no accepted offer in N days)          └→ CANCELLED (seller may republish)
any → WITHDRAWN (seller)   any → BLOCKED (ops/moderation)
```
- `NEEDS_INPUT` means the AI asked for proof or a photo retake; the seller can always skip to `READY`.
- Editing a published listing creates a new `listing_version`; quotes, matches and cache keys are scoped to the version.

### 4.3 Offer states
`ACTIVE → ACCEPTED | REJECTED | WITHDRAWN | EXPIRED | SUPERSEDED (by counter)`. Accepting one offer auto-rejects the others on that listing version.

### 4.4 Deal states
```
CREATED → PICKUP_SCHEDULED → (JOB_ASSIGNED) → AT_PICKUP → WEIGHED → SELLER_CONFIRMED
   → AWAITING_PAYMENT (Mode A) → PAID → HANDED_OVER → IN_TRANSIT → DELIVERED → SETTLED → CLOSED
Mode B (direct payment): WEIGHED → SELLER_CONFIRMED → PAID_DIRECT (seller confirms receipt) → HANDED_OVER → ...
Branches: SELLER_NO_SHOW, PICKUP_NO_SHOW, LINE_REGRADED (item differs → re-priced; seller accepts or removes the line),
          DISPUTED (→ ops), CANCELLED (reason code + fee policy)
```
Deal lifecycles run as **AWS Step Functions Standard workflows** whose task states invoke Rust Lambdas.
- **Why:** they last hours to days and need durable timers (offer expiry, transporter acceptance timeout, payment SLA, auto-close) with retries and full execution history.
- **Goal:** no deal stuck in an intermediate state without a timer or an owner.

### 4.5 Transport job states
`OFFERED → ACCEPTED | DECLINED | TIMED_OUT → EN_ROUTE → ARRIVED_PICKUP → LOADED → IN_TRANSIT → ARRIVED_YARD → DELIVERED → PAID_OUT`. On timeout, the offer moves to the next transporter in the vendor's ranked list.

---

## 5. Identity and sign-in (AWS-native)

Applies to all roles and all clients.

### 5.1 Phone OTP: WhatsApp first, SMS fallback
```
Client ──InitiateAuth(USER_AUTH, preferred challenge SMS_OTP)──► Amazon Cognito
                                                                   │ generates code, encrypts with our KMS key
                                                                   ▼
                                              Custom SMS Sender trigger (Rust Lambda)
                                                 │ decrypt code (AWS Encryption SDK + KMS)
                                                 │ read channel preference (DynamoDB, TTL)
                                                 ├─► AWS End User Messaging Social → WhatsApp authentication template
                                                 └─► AWS End User Messaging SMS (DLT entity + template IDs)
Client ──RespondToAuthChallenge(code)──► Cognito verifies → ID/access/refresh tokens
```
- **What:**
  - **Amazon Cognito user pool** with passwordless sign-in (`USER_AUTH` flow, SMS OTP challenge); Cognito generates, expires and verifies every code.
  - A **Custom SMS Sender Lambda trigger** (Rust) receives the encrypted code and delivers it:
    - first via **AWS End User Messaging Social** (WhatsApp authentication template, sent from the Mumbai region);
    - via **AWS End User Messaging SMS** (India DLT Entity ID and Template ID) when the user taps "Send by SMS instead", when WhatsApp delivery fails (delivery events arrive through SNS), or when the number isn't on WhatsApp.
  - The resend choice is stored as a short-lived channel preference in DynamoDB (keyed by phone hash, TTL a few minutes) by a `POST /v1/auth/otp-channel` call before resending; the trigger reads it.
- **Why:**
  - Codes never touch our database; Cognito owns generation, expiry, retry limits and token issuance.
  - WhatsApp authentication messages are cheaper and more reliable than SMS in India; SMS keeps sign-in working for everyone else.
  - Everything stays on AWS (one bill, IAM-controlled, data in ap-south-1), and AWS documents this Cognito + WhatsApp pattern ([AWS Security Blog](https://aws.amazon.com/blogs/security/how-to-use-whatsapp-to-send-amazon-cognito-notification-messages)).
- **Goal:** sign-in in < 60 seconds; OTP delivery success > 98%.
- **Abuse protection:** AWS WAF rate-based rules and CAPTCHA/Challenge on the auth endpoints, Cognito threat protection, per-number throttles in the trigger, and CloudWatch alarms on messaging spend.

### 5.2 Clients
- **Websites:** the AWS Amplify JavaScript library's **Auth module only** (client-side), configured for the `USER_AUTH` passwordless flow.
- **Transporter app:** the AWS SDK for Rust Cognito client (`InitiateAuth` / `RespondToAuthChallenge` on a public app client, no secret); refresh token stored in the Android Keystore through the native bridge (§8.2).
- **API:** API Gateway JWT authorizer validates Cognito access tokens; Rust services read claims and resolve permissions through Cedar (§18).

### 5.3 Profile and address
- **What:** after OTP: name → address with map pin and landmark → 18+ confirmation.
- **Address entry:** **Amazon Location Service Places** (`Autocomplete`, `Geocode`, `ReverseGeocode`) for search and pin-drop; map tiles from **Amazon Location Maps** rendered with MapLibre GL JS. Places calls go through the Rust API (cached, rate-limited, no keys in clients).
- **Why:** Amazon Location keeps geodata processing on AWS; the landmark field compensates for irregular Indian addresses; pin-drop gives an exact point even when the text address is vague.
- **Stored:** lat/lng plus H3 cells (§12.5); the free-text address and landmark are field-level encrypted.
- **18+ only:** DPDP requires verifiable parental consent for children's data; the service is adults-only.

---

## 6. Seller website

**Next.js static export** (`output: 'export'`): no Server Components data fetching, Server Actions, Route Handlers, middleware or ISR. Pages are pre-built HTML/JS shells; all data loads client-side from the Rust API with TanStack Query. Installable PWA (service worker with offline drafts and an upload queue). Hosted on AWS Amplify Hosting as a static site.
- **Why static:** no server runtime to secure, scale or pay for; the Rust API is the single backend; CDN-served pages load fast on low-end phones.
- **Routing:** per-resource pages use query-parameter routes (`/listing?id=…`), because static export can't pre-render per-ID pages; Amplify Hosting rewrite rules handle deep links.

### 6.1 Create a listing
1. **Guided photo capture**
   - **What:** camera capture (`<input type="file" accept="image/*" capture="environment">` and `getUserMedia` where available) with prompts: one wide shot of everything, then close-ups per item; client-side quality hints plus server-side checks (blur, darkness, item cut off). Images are resized client-side and uploaded straight to S3 with presigned URLs.
   - **Why:** photo quality is the biggest driver of AI accuracy and vendor trust; direct-to-S3 uploads keep large files off the API.
   - **Goal:** < 10% of listings need a retake request.
   - Gallery uploads are allowed but labelled "uploaded from gallery"; proof photos must be camera captures (§14).
2. **Describe (optional text)**
   - **What:** optional free text ("two old ACs, one fridge and some newspapers").
   - **Why:** descriptions add context photos can't show (working condition, age, brand) and carry claims the AI should check.
   - **Goal:** descriptions present on ≥ 50% of listings.
3. **AI item list**
   - **What:** the AI returns items with category, attributes (material, condition, brand/model if visible), count, plausible weight range, hazard flags and handling tips; progress streams over AppSync Events. The seller can edit, add, split or merge items.
   - **Why:** removes the "I don't know what this is called or worth" barrier.
   - **Goal:** sellers accept AI categories unchanged for ≥ 80% of items.
4. **Estimated weight (required)**
   - **What:** per item or total. Helpers come from an ops-curated reference library ("ceiling fan: typically X–Y kg"), not from the AI. If an estimate falls outside the AI's plausible range, a gentle note appears; the system never blocks or overwrites the seller's number.
   - **Why:** weight drives value, vendor fit and vehicle choice; seller ownership of the number keeps accountability clear.
   - **Goal:** median absolute weight error (estimate vs. scale) tracked and trending down.
5. **Price: suggestion vs. claim**
   - **What:** a **suggested range** per item and in total, computed from live quotes of matching vendors plus the Rate Index (§12.6). The seller accepts it, or enters an **asking price** with a **claim reason** ("branded copper winding, unused").
   - **Why:** the seller decides; the suggestion anchors expectations to real market data.
   - **Goal:** fewer listings without offers due to unrealistic asks; ask-to-deal-price ratio tracked.
6. **Proof requests (optional for the seller)**
   - **What:** the verification agent may ask for specific evidence ("close-up of the cut end of the coil", "photo of the label with the model number"). Max 3 requests per round, max 2 rounds. Each request states the benefit ("vendors pay a verified bonus for clean copper").
   - **Seller options:** provide proof, skip, or contest ("the AI is wrong"). Contested claims are shown to vendors as "seller disputes AI assessment".
   - **Why:** proof up front saves every vendor from asking separately and earns the seller better offers.
   - **Goal:** ≥ 50% of high-value claims reach ladder level L2 or higher; fewer pre-deal chat messages per deal.
7. **Pickup preferences**
   - **What:** preferred days/slots, floor and lift availability, item location (terrace, ground floor), parking access.
   - **Why:** vendors and transporters price effort; surprises cause no-shows.
8. **Review and publish**
   - **What:** summary with verification badges, hazard notes and the number of currently matching vendors.
   - **Goal:** ≥ 70% of started listings get published.

### 6.2 Offers, deal and pickup
- **Compare offers:** total (range from the estimate), per-category rates, vendor rating, distance, pickup slot, verification bonus applied, "pays via platform" badge; sorted by total.
  - **Why:** comparison is where sellers see the value of a marketplace.
- **Counter-offer** once, or chat. After acceptance, parties talk through **in-app calls** (§19.3); phone numbers are never shown.
- **Accept** → deal created → seller sees vendor/transporter ETA and live location during the active job only.
- **At pickup:** the seller sees the scale photo and computed amount and confirms or disputes; in Mode A they see "payment received" before giving the **handover OTP**.
  - **Why:** the seller never hands over goods unpaid; the OTP proves a real handover happened.
- **After:** Green Receipt (PDF + shareable link), rating, impact counter.

### 6.3 Seller account
History, receipts, payout methods (UPI ID validated before the first payout), saved addresses, notification preferences, data export and account deletion (DPDP rights), help and disputes.

---

## 7. Vendor website

Same static-export Next.js architecture as the seller website (§6). Mobile-first PWA with large touch targets, icon-led navigation and short plain labels; Web Push for first-look leads.

### 7.1 Onboarding and KYC
- **What:** phone OTP (§5) → business name, yard location (Amazon Location pin), yard and scale photos → PAN (and GSTIN if registered) → payment details → categories handled → service area.
- **Document processing:** **Amazon Textract** (`AnalyzeDocument` with Queries) extracts PAN/GSTIN/authorization numbers and names from uploaded documents; extracted values are verified against government records through the KYC provider adapter (§15.4) and reviewed by ops on mismatch.
- **Hazardous category unlock:** upload SPCB authorization (or a documented tie-up with an authorized recycler) → Textract extraction → ops verification against the authorized-recycler items (`RECYCLER#…`) → expiry tracked; categories auto-lock on expiry.
- **Why:** trust for sellers, legal routing of e-waste and batteries, payout compliance; Textract removes manual typing from KYC review.
- **Goal:** KYC decision within 24 hours; zero hazardous offers from unauthorized vendors.

### 7.2 Pricing rules (rate card)
- **What:** a rate card per category and grade (₹/kg or ₹/piece), plus:
  - service area (radius or drawn polygon), working days and slots, daily capacity;
  - minimum order value and minimum weight;
  - modifiers: verified-item bonus %, unverified discount %, distance deduction (₹/km after N free km), bulk tiers, no-lift surcharge per floor;
  - exclusions ("no glass", "no mixed plastic").
- **Three ways to edit:**
  1. form editor;
  2. **plain-text instruction** typed into a box ("copper 650, brass 400, paper 14 above 5 kilo, no glass"), drafted into rules by the rules-authoring agent (§11.7);
  3. **bulk change** ("all metals +3% from tomorrow 9am").
- **Simulate before publishing:** shows how the new card would have priced the last 20 nearby listings and how matches would change.
- **Versioning:** every publish creates an immutable `rule_set_version` with `effective_from`; deals keep the version they were priced with.
- **Why:** scrap prices move often (especially metals); vendors need fast updates and we need auditable prices.
- **Goal:** ≥ 70% of active vendors update their card at least weekly; zero quotes from unpublished drafts.

### 7.3 Listings feed and matches
- **Matches for you:** listings ranked for this vendor by the matching engine, with the computed quote, coverage ("covers 4 of 5 items") and distance. Top-K vendors get a **first-look window** (configurable, e.g., 20–30 min) before the listing opens to everyone.
  - **Why:** rewards vendors who keep accurate rate cards; gives sellers fast offers.
- **Browse all:** filters by category, distance, minimum value, verification level, pooled drives, hazardous (if authorized); list and map views (Amazon Location Maps). Location appears as an approximate area (H3 cell), never the exact address.
- **Listing detail:** all photos (faces blurred, EXIF stripped), AI item list, verification ladder per claim, proof history, seller's asking price and reason, seller honesty badge, pickup notes.

### 7.4 Offers, chat and deals
- **Make offer:** pre-filled from the vendor's rate card (deterministic quote) and editable per line; validity; proposed slot; pickup method (self or transporter). Edits away from the rate card are logged.
- **Chat:** text chat over AppSync Events; phone numbers and UPI IDs typed into pre-deal chat trigger a soft warning (protects against scams and off-platform deals).
- **Deal board:** today's pickups, status per deal, payments due, disputes; in-app call button per active deal.

### 7.5 Transporter selection
- **What:** "My transporters" (invited by phone; trusted list) ranked by ETA (Amazon Location route matrix), quoted fare, vehicle fit and reliability; or nearby marketplace transporters. Offer the job to one transporter at a time with a timeout, or broadcast to the trusted list (first accept wins).
- **Why:** vendors already rely on trusted drivers; the app digitizes that relationship.
- **Goal:** median time from deal to transporter acceptance < 15 min.
- **Fare:** agreed per job (fixed, or a per-km template saved per transporter); paid by the vendor (§13.3).

### 7.6 Vendor business tools
Procurement ledger (kg and ₹ by category and day), receipts, payment history, ratings, compliance documents, and for authorized vendors a chain-of-custody export for hazardous lots.

---

## 8. Transporter app (Dioxus)

### 8.1 Architecture
- **Framework:** **Dioxus 0.7** mobile, built with the `dx` CLI (`dx serve --platform android`, `dx bundle --platform android --release`). App logic is Rust compiled for Android; the UI renders in the system WebView.
- **Shared Rust crates:** `kabados-domain` (types, state machines), `kabados-client` (typed API client generated from the backend's OpenAPI), `kabados-pricing` (to show the computed amount at the scale instantly, even offline; the server recomputes the authoritative amount).
  - **Why:** one language and one set of types from database to driver's phone; no client/server drift in states or money math.
- **Local storage:** SQLite (`rusqlite`) for the offline event and photo queue; tokens in the Android Keystore.
- **Networking:** `reqwest` (rustls) to the API; AppSync Events WebSocket for live job offers and status while on duty.

### 8.2 Native Android bridge
Dioxus exposes Android through JNI. Platform features without a Rust/WebView equivalent live in a small Kotlin module (`apps/transporter/android-bridge`) merged into the Gradle project that `dx bundle` generates, and called from Rust via the `jni` crate.

| Capability | Native implementation | Why native |
|---|---|---|
| Job-offer push | Firebase Messaging service receiving **high-priority FCM data messages** sent by **Amazon SNS mobile push** | Android delivers wake-up pushes only through FCM; SNS keeps the sending side on AWS. |
| On-duty connection | **Foreground service** (persistent "On duty" notification) holding the AppSync Events WebSocket | Driver apps need sub-second offers while on duty; Android stops background sockets without a foreground service. |
| Incoming-job screen | Full-screen intent notification with sound | Offers must interrupt whatever the driver is doing. |
| Background location | Fused location provider inside the foreground service, active only during an accepted job | Accurate GPS for geofences without draining battery when idle. |
| Proof camera | CameraX capture activity (no gallery access) | Guarantees in-app capture and trustworthy capture metadata. |
| Secure storage | Android Keystore-backed encrypted storage | Refresh tokens and offline OTP keys must not be readable by other apps. |
| In-app calls | Amazon Chime SDK for Android | Audio calls without exchanging phone numbers (§19.3). |
| Device integrity | Play Integrity API token | Lets the backend reject emulators and tampered builds (§14). |

- **Why a bridge:** Dioxus's Android support is still maturing and has no first-party plugins for these features; isolating them behind a narrow bridge interface (`start_duty()`, `capture_photo()`, `current_location()`…) keeps the rest of the app pure Rust and testable on desktop.

### 8.3 Onboarding
- **What:** phone OTP (§5) → driving licence, vehicle RC, selfie, vehicle photos, vehicle type and payload → payout details → optional insurance.
- **Verification:** **Amazon Textract** extracts DL/RC numbers and names; **Amazon Rekognition** `CompareFaces` matches the selfie to the licence photo; numbers are verified against government records through the KYC provider adapter; ops reviews mismatches.
- **Why:** sellers let these people into their homes; vendors trust them with goods.

### 8.4 Jobs
- **Availability toggle** ("On duty") starts the foreground service; only on-duty transporters receive offers.
- **Incoming job:** full-screen alert showing approximate pickup area (exact after acceptance), distance, load summary (kg, bulky items, floors), fare, vendor name and rating, and an accept/decline **countdown**.
- **Call vendor:** in-app call to negotiate fare or timing; any fare change is recorded on the job.
- **Active job checklist** (each step timestamped and GPS-tagged):
  1. navigate (intent to the device's maps app; route preview from Amazon Location Routes);
  2. arrive (Amazon Location geofence event; manual fallback);
  3. photos of items as found;
  4. weigh: photo of the scale display with the item on it; AI reads the number; driver confirms or corrects; the app shows the computed amount from `kabados-pricing`;
  5. wait for seller confirmation (and payment in Mode A);
  6. enter the seller's **handover OTP**;
  7. in transit;
  8. arrive at the vendor yard (geofence) → vendor confirms receipt.
- **Live location:** positions are posted to the API, which writes them to an **Amazon Location tracker** (`BatchUpdateDevicePosition`); geofence enter/exit events arrive through EventBridge. Shared only from acceptance to delivery.
- **Offline tolerance:** photos and events queue in SQLite and sync when the network returns; the handover OTP can be verified offline with a short-lived signed code.
- **Earnings:** per job, per day, payout status.
- **Why:** each step creates evidence that makes disputes rare and resolvable.
- **Goal:** ≥ 80% of offers accepted within 2 minutes; ≥ 95% of jobs with complete evidence.

---

## 9. Pooled collection drives

- **What:** a society admin creates a drive (date, collection point, categories). Residents join via link or QR and add items as listings inside the drive. Vendors see the **pool total** and bid on the whole drive with per-category rates. One pickup; each household's items are weighed and paid separately.
- **Why:** a single household's few kilos aren't worth a trip; a building's combined scrap is.
- **Goal:** pooled listings get offers ≥ 3× more often than solo listings under 10 kg.
- **Details:**
  - Per-household settlement uses the same pricing crate and ledger as normal deals.
  - Drive report: kg by category, hazardous kg routed to authorized vendors, participation rate; downloadable for the society or a bulk waste generator's records.
  - Recurring drives (e.g., monthly) with reminders, scheduled with EventBridge Scheduler.

---

## 10. Ops console and public pages

### 10.1 Ops console
Next.js static export behind Cognito (separate staff user pool with mandatory MFA).

| Module | Purpose |
|---|---|
| KYC queue | Approve/reject vendors and transporters with Textract-extracted fields side by side; hazardous authorizations with expiry tracking. |
| AI review queue | Low-confidence assessments, high-value claims above a threshold, contested AI verdicts. Reviewer decisions feed the evaluation sets. |
| Disputes | Evidence timeline per deal (photos, scale readings, GPS, chat, payments); actions (refund, re-price, warn, suspend); SLA timers. |
| Taxonomy and reference weights | Categories, grades, hazard flags, e-waste schedule mapping, per-piece weight ranges, handling instructions. |
| Fraud | Duplicate-image clusters, device/account links, abnormal patterns, mock-location hits. |
| Rate Index | Review and publish the weekly index; outlier handling. |
| Config | Feature flags, first-look window, matching weights, fee policy, city enablement (AWS AppConfig). |
| Audit log | Every admin action, append-only. |

### 10.2 Public pages
Next.js static export with **data fetched at build time**; a weekly EventBridge rule (after the Rate Index is published) triggers a CodePipeline rebuild and deploy.
- **Why:** search engines get fully rendered pages without any server runtime.
- **Kabad Rate Index:** weekly median ₹/kg per category per city, published only above a k-anonymity threshold (e.g., ≥ 20 deals from ≥ 5 vendors) so no individual vendor's prices are exposed.
- **City impact:** kg diverted by category; hazardous kg routed to authorized recyclers; estimated emissions avoided, with the methodology linked (§11.11).
- **Green Receipt verification:** a static page that calls the public API to confirm a receipt's signed ID.

---

## 11. AI system

### 11.1 Principles
1. **AI informs; deterministic code decides money and permissions.** Prices come from the pricing crate, access from Cedar policies, settlement from the scale reading.
2. **Every AI output is schema-validated**, versioned (model + prompt version) and stored with its inputs for audit and evaluation.
3. **Seller and vendor text is untrusted data, never instructions** (prompt-injection defence, §11.9).
4. **Neutral language:** the AI never says "false" or "fraud"; it says "not enough evidence yet" and asks for something specific.
5. **Fast path first:** plain code handles the common case; agents handle ambiguity, which controls cost and latency.

### 11.2 Models, client library and agent runtime
- **Google Gemini API** (existing paid API subscription), called from Rust with the **`gemini-client-api`** crate (v7.5.x, MIT, async on Tokio + `reqwest`/rustls).
  - **API reference:** the library source is checked out at `gemini-client-api/llms-client/gemini` (README, `examples/`, `src/gemini/`); it is the reference for all AI integration code.
  - **Why:** the team already has a paid Gemini API plan; the crate is native Rust (fits the Rust-only backend and Lambda), and covers everything the agents need: multimodal input (images, PDFs), JSON mode with schemas generated from Rust types, function calling, thinking mode, context caching and streaming.
- **Library features used:**
  - `Gemini` client (`Gemini::new(api_key, model, system_instruction)`), with `set_sys_prompt`, `set_safety_settings`, `set_thinking_config` and `new_with_timeout` for per-agent configuration.
  - `Session` for the conversation history of an agent run (bounded history length; `set_remember_reply(false)` for single-shot calls).
  - **Structured output:** `#[gemini_schema]` on the Rust output structs + `set_json_mode(T::gemini_schema())`, then `response.get_json::<T>()` and our own validation.
  - **Tools:** `#[gemini_function]` on Rust tool functions, registered with `set_tools(vec![Tool::FunctionDeclarations(...)])`, dispatched with `execute_function_calls!`.
  - **Multimodal:** listing photos (downscaled, EXIF-stripped, faces blurred) attached as inline image parts of the session message.
  - **Context caching:** `create_cache` / `set_cached_content` for the large, stable prompt prefix (taxonomy, grade definitions, reference weights, instructions), refreshed when the taxonomy changes.
- **Model tiering (model IDs in AppConfig, not code):** a Flash-class Gemini model for photo triage, scale reading and simple classification; a Pro-class model with thinking enabled for itemization, claim verification, matching and rule authoring; the strongest available model for offline evaluation and hard review cases.
  - **Why:** cost and latency proportional to task difficulty; switching models is a configuration change.
- **Agent runtime (`kabados-agent` crate):** a thin layer over `gemini-client-api` that adds what the library leaves to the caller:
  - step, token and wall-clock budgets per run; retries with backoff on rate-limit and 5xx errors;
  - schema validation of every JSON output with one repair retry;
  - a `tracing` span per model call and tool call (model, tokens, latency) exported to X-Ray;
  - tools are in-process Rust functions (pricing crate, DynamoDB repositories, Cedar checks), so tool calls add no network hop.
- **Credentials:** the Gemini API key lives in AWS Secrets Manager, is read once per Lambda container and is never sent to clients.
- **Runtime:** Rust Lambdas; async agents (triage, itemization, verification, scale reading) consume SQS; the matching agent runs synchronously behind the matching API with a hard timeout.
### 11.3 Agent 1: Photo triage
- **Input:** each uploaded image.
- **Output:** `{usable: bool, issues: [BLUR|DARK|CUT_OFF|NO_ITEMS|PEOPLE_VISIBLE|DUPLICATE], retake_hint}`.
- **Why:** catches bad photos in seconds, before the expensive model runs and while the seller is still on the page.
- Runs alongside **Amazon Rekognition** `DetectModerationLabels` and `DetectFaces`; faces are blurred in vendor-visible copies.

### 11.4 Agent 2: Itemization
- **Input:** all usable images of a listing version, seller description (as data), taxonomy snapshot, reference-weight table.
- **Output (per item):**
```json
{
  "item_id": "tmp-1",
  "category_id": "ewaste.ceew.split_ac_outdoor",
  "attributes": {"material": "mixed", "brand": null, "condition": "non-working-unknown"},
  "quantity": 1,
  "unit": "PIECE",
  "plausible_weight_g": {"min": 0, "max": 0, "source": "REFERENCE_TABLE | MODEL_ESTIMATE"},
  "hazard": {"is_hazardous": true, "class": "EWASTE_REFRIGERANT", "handling": "..."},
  "evidence": [{"image_id": "img_3", "bbox": [0.1, 0.2, 0.5, 0.7]}],
  "confidence": 0.0
}
```
- **Rules:** weight ranges come from the ops-curated reference table when the category has one; model estimates are flagged `MODEL_ESTIMATE` and shown as low-confidence. The agent never outputs prices.
- **Why:** turns photos into the structured data every other component needs.
- **Goal:** top-1 category accuracy ≥ 90% on the evaluation set; **hazard recall ≥ 99%** (missing a battery is worse than a false alarm).

### 11.5 Agent 3: Claim verification and proof requests
- **Input:** itemization, images, seller claims (edited item list, description, asking price + claim reason), grade definitions, prior proof rounds; **Amazon Textract** text detected on labels and model plates.
- **Output (per claim):**
```json
{
  "claim_id": "c-2",
  "claim_text_ref": "seller_description#span-14-32",
  "verdict": "SUPPORTED | INSUFFICIENT_EVIDENCE | INCONSISTENT",
  "ladder_level": "L1_PHOTO_CONSISTENT",
  "rationale": "The coil is visible but the winding colour can't be confirmed from this distance.",
  "proof_requests": [
    {"shot_type": "CLOSE_UP | LABEL | CUT_SECTION | COMPONENT_SIDE | WORKING_DEMO | SCALE",
     "instruction": "Take a close-up of the cut end of the coil in daylight.",
     "benefit": "Vendors pay a verified bonus for clean copper."}
  ],
  "confidence": 0.0
}
```
- **Verification ladder:**

| Level | Meaning | Set by |
|---|---|---|
| L0 Claimed | Seller said so; no supporting image | Default |
| L1 Photo-consistent | Photos are consistent with the claim | AI |
| L2 Close-up evidence | Close-up shows material, colour or grade | AI |
| L3 Identifier evidence | Label, model number or markings read (Textract) and consistent | AI |
| L4 Verified at pickup | Vendor or transporter confirmed physically | Human, at pickup |

- **Why a ladder:** it rewards proof without accusing anyone; vendors see exactly how much is verified, and AI mistakes don't brand honest sellers.
- **Deterministic post-checks:** `SUPPORTED` requires at least one evidence image reference; the ladder moves up only from images, never from text; max 3 requests per round, 2 rounds.
- **Goal:** ≥ 90% of proof requests rated "useful and specific" by reviewers; pickup item-mismatch < 10% overall, < 3% for L2+ items.

### 11.6 Agent 4: Matching orchestrator
Tools exposed to the agent (in-process calls into deterministic code):

| Tool | Does |
|---|---|
| `get_listing_snapshot(listing_version_id)` | Items, weights, ladder levels, location cell, pickup preferences |
| `find_candidate_vendors(h3_cells, category_ids, window)` | Reads the coverage index (§12.5) |
| `quote(listing_version_id, vendor_ids)` | Calls `kabados-pricing`; returns per-vendor quote, coverage, explanation lines |
| `vendor_signals(vendor_ids)` | Reliability, rating, response time, current load |
| `widen_search(step)` | Next ring of H3 cells, within policy limits |
| `suggest_split(listing_version_id)` | Splits a mixed lot across specialist vendors (e.g., e-waste to an authorized recycler, paper to a kabadiwala) |
| `suggest_proof_unlocks(listing_version_id)` | Finds proof that would raise the ladder level and unlock verified bonuses |

- **When it runs:** only when the fast path (§12.7) returns fewer than N matches, low coverage, or a mixed lot containing hazardous items.
- **Why:** sparse areas, mixed lots and "what would get me a better offer?" need multi-step reasoning over tools; the common case doesn't, so it stays on the cheap path.
- **Output:** a validated `MatchResult` (top-N vendors with quotes, coverage and reasons; optional split plan; optional proof suggestions for the seller).

### 11.7 Agent 5: Rules authoring
- **Input:** the vendor's typed instruction, current rule set, taxonomy, Rate Index.
- **Output:** a `RuleSetDraft` (same schema as §12.2) plus ambiguities to confirm ("Does 'paper' mean newspaper only, or cardboard too?").
- **Guardrails:** schema validation; every category maps to the taxonomy; prices far from the Rate Index (e.g., ±40%) trigger a confirmation; **nothing publishes without the vendor tapping Publish** on a preview.
- **Why:** vendors keep accurate rate cards by typing one sentence instead of filling a form.
- **Goal:** ≥ 50% of rule updates via the text box.

### 11.8 Agent 6: Scale reading
- **Input:** scale photo (7-segment, LCD or analogue display).
- **Output:** `{reading_g, unit_detected, confidence, display_bbox}`; on low confidence the driver types the value and the photo is kept as evidence.
- **Why:** removes typing errors and makes every weight auditable.

### 11.9 Safety, guardrails and prompt injection
- Seller descriptions, chat messages and claim reasons are passed as quoted data; system prompts state they may contain instructions that must be ignored.
- **Gemini safety settings** (`set_safety_settings`) block harassment, hate, sexual and dangerous content in inputs and outputs.
- A Flash-class **prompt-attack and abuse classifier** (JSON-mode call) screens seller text and chat messages before they reach other agents or users; flagged content is quarantined for ops review.
- **PII minimization before any API call:** phone numbers, UPI IDs and addresses are redacted from text by our own Rust filters; images are downscaled, EXIF-stripped and face-blurred (Rekognition) before upload; logs store redacted prompts only.
- Every output is deserialized and validated; one automatic repair retry, then fail closed (listing moves to `READY` without AI verdicts and is flagged for ops review).
- Agents have read-only tools plus "write draft" tools; they cannot publish listings, send offers, move money or change permissions.
- Images are passed by S3 reference within the account; only downscaled copies go to the model; original EXIF stays in the backend.

### 11.10 Evaluation and ground-truth loop
- **Golden sets** per capability: itemization, hazard detection, claim verdicts, proof-request quality, scale reading, rules authoring; seeded from ops-labelled data and extended with production labels. Manifests in the repo, data in S3.
- **Labels from pickups:** every completed deal yields actual category and grade (vendor-confirmed), weighed weight and the L4 outcome. Images and labels enter evaluation pools only under the consent recorded for that user (§20.2).
- **Eval runner:** a Rust CLI (`kabados-evals`) that replays golden sets through the same agent code and writes reports to S3; runs in CodeBuild on every prompt, model or schema change. Regressions in hazard recall or category accuracy block the release.
- **Shadow mode:** a candidate model runs in parallel on a traffic sample before it replaces the current one.
- **Live monitoring:** mismatch at pickup, contested-verdict rate, retake rate, token cost per listing, latency p95 (CloudWatch metrics).

### 11.11 Environmental impact accounting
- kg by category from scale readings, never from estimates.
- Emissions avoided = kg × category emission factor from a cited public dataset (e.g., US EPA WARM, or India-specific factors where published); shown as an **estimate** with a methodology page, never as certified carbon credits.
- Hazardous chain of custody: seller → authorized vendor → downstream recycler (recorded by the vendor).

### 11.12 Cost controls
Images downscaled to the model's recommended resolution; AI outputs cached by image hash + prompt version; fast path before agents; Flash-class triage; context caching for the stable prompt prefix; per-listing token budget with alarms; token usage metrics in CloudWatch and billing alerts on the Gemini API account.

---

## 12. Pricing rules, quoting and matching

### 12.1 Taxonomy
Hierarchical categories with stable IDs (e.g., `paper.newspaper`, `paper.cardboard`, `plastic.pet_bottle`, `metal.copper`, `metal.brass`, `metal.aluminium`, `metal.iron_steel`, `ewaste.itew.laptop`, `ewaste.ceew.refrigerator`, `battery.lead_acid`, `battery.li_ion`, `glass.mixed`, `textile.mixed`). Each category has:
- unit (`KG` or `PIECE`) and allowed grades (e.g., copper: `clean`, `mixed`, `with_insulation`);
- hazard flag and hazard class; mapping to E-Waste Rules schedule categories where applicable;
- reference weight range for per-piece items; handling instructions.

### 12.2 Rule set schema (vendor rate card)
Money in **paise** (integer), weights in **grams** (integer). Prices below are placeholders, not market rates. The schema is a Rust type in `kabados-domain` (serde + schemars); its JSON Schema is published for clients.
```json
{
  "rule_set_id": "rs_01J...",
  "version": 7,
  "effective_from": "2026-10-12T09:00:00+05:30",
  "service_area": {"type": "RADIUS", "center": {"lat": 28.61, "lng": 77.21}, "radius_km": 8},
  "schedule": {"days": ["MON", "TUE", "WED", "THU", "FRI", "SAT"], "slots": ["09:00-13:00", "14:00-19:00"], "daily_capacity_kg": 800},
  "minimums": {"order_value_paise": 20000, "weight_g": 5000},
  "lines": [
    {"category_id": "metal.copper", "grade": "clean", "price_per_kg_paise": 0},
    {"category_id": "metal.copper", "grade": "mixed", "price_per_kg_paise": 0},
    {"category_id": "paper.newspaper", "price_per_kg_paise": 0, "min_weight_g": 5000},
    {"category_id": "ewaste.ceew.split_ac_outdoor", "unit": "PIECE", "price_per_piece_paise": 0,
     "conditions": {"compressor_present": true}}
  ],
  "modifiers": [
    {"type": "VERIFIED_BONUS_PCT", "min_ladder": "L2_CLOSE_UP", "value": 3},
    {"type": "UNVERIFIED_DISCOUNT_PCT", "max_ladder": "L0_CLAIMED", "value": 5},
    {"type": "DISTANCE_DEDUCTION", "free_km": 3, "per_km_paise": 500},
    {"type": "BULK_TIER", "category_glob": "paper.*", "threshold_g": 50000, "bonus_pct": 5},
    {"type": "NO_LIFT_SURCHARGE_PER_FLOOR_PAISE", "value": 0}
  ],
  "exclusions": ["glass.*"],
  "accept_partial_lots": true
}
```
Validation: serde deserialization plus semantic checks (no overlapping lines of equal specificity, categories exist, hazardous categories only for authorized vendors, price bounds).

### 12.3 Quote algorithm (pure, deterministic, versioned)
For listing version `L` and vendor rule set version `R`:
1. **Eligibility:** vendor active; listing cell inside the service area; pickup window overlaps the schedule; capacity left; hazardous items only if authorized (Cedar, §18).
2. **Line matching:** for each item, the most specific rule line (category + grade + conditions); ties broken by line order. Items without a line are uncovered (allowed if `accept_partial_lots`).
3. **Weight range:** `w_low..w_high` = seller estimate ± category tolerance (e.g., ±15%), intersected with the AI plausible range when that range comes from the reference table.
4. **Line value:** `rate × w` (or `price_per_piece × qty`) for low and high, using integer paise arithmetic with explicit rounding rules.
5. **Verification modifiers** per item from its ladder level.
6. **Lot modifiers:** bulk tiers, distance deduction (road distance from the yard via Amazon Location Routes, cached per cell pair), floor surcharge.
7. **Minimums:** below minimum order value or weight → ineligible (reported, so the agent can suggest pooling).
8. **Output:** `{eligible, covered_item_ids, coverage_ratio, quote_low_paise, quote_high_paise, lines:[...], explanation_codes:[...], engine_version, rule_set_version}`.

Implemented once as the pure Rust crate **`kabados-pricing`** (no I/O), used by the API Lambdas, workers, agents and the transporter app. The **same function** computes listing suggestions, offer pre-fill, re-grades at pickup and final settlement (weighed weight used as both low and high).
- **Why:** one implementation guarantees that what a seller is shown, what a vendor offers and what is paid can never drift apart; integer arithmetic avoids floating-point money errors.
- **Tests:** `proptest` property tests (monotonic in weight, bounded modifiers, never negative, rounding stable) and golden tests per rule feature.

### 12.4 Ranking
```
score(v) = w_price   · norm(expected_payout_mid)
         + w_cov     · coverage_ratio
         + w_dist    · (1 − distance / max_radius)
         + w_rel     · reliability (on-time %, completion %, payment speed)
         + w_rating  · bayesian_rating
         + w_resp    · response_speed
         + exploration_bonus (new vendors, capped share of impressions)
```
- Weights live in AWS AppConfig and are tuned with A/B tests on offer rate and deal conversion.
- Hard filters (eligibility, hazardous authorization, suspension) apply before scoring, never as weights.
- **Why the exploration bonus:** new vendors otherwise never get leads, and supply stalls.

### 12.5 Coverage index (cache level 1)
- Locations are indexed with **H3** (`h3o` crate): resolution 8 for matching (roughly neighbourhood-sized), resolution 7 for the vendor feed and display.
- Coverage items `PK = COV#<h3_r8>#<categoryId>`, `SK = VENDOR#<vendorId>` (attributes: rule set version, line reference) are rewritten for a vendor on every `RuleSetPublished` event and on KYC/authorization changes, by a Streams-triggered worker.
- Matching issues one `Query` per (cell, category) pair in the search ring, in parallel.
- **Why:** key lookups replace a geospatial scan over all vendors, so matching cost stays flat as the vendor base grows, and it fits DynamoDB's key-value model.

### 12.6 Suggested price for sellers
- Per item: median and inter-quartile range of quotes from the top eligible vendors (top-10 by score), blended with the Rate Index when fewer than 3 vendors quote; with neither, the UI shows "no price data yet".
- Displayed as a range with "based on N vendors near you". A seller's asking price overrides it (§6.1); vendors whose quote meets the ask are highlighted to the seller.

### 12.7 Matching pipeline and caching
```
Publish/edit ─► MatchingRequested(listing_version)        (Rust matching Lambda)
   1. Fast path:
        cells = ring(h3_r8, k)            # k from listing size and city density
        candidates = coverage_index(cells, categories)
        versions = {vendor_id: rule_set_version}
        key = sha256(listing_version_id, cells, sorted(versions))
        if cache.get(key): return it                    # cache level 2
        quotes = pricing::quote_many(listing, candidates)
        ranked = rank(quotes, signals)
        if good_enough(ranked): cache.set(key, ranked, ttl=6h); return
   2. Slow path (matching agent, §11.6): widen, split, suggest proofs
        result → validate → cache.set(key, result, ttl=6h)
   3. Notify top-K vendors (first-look window), then open to the feed
```
- **Why rule-set versions are in the cache key:** when any candidate vendor changes prices, the key changes automatically, so no invalidation logic can serve stale prices.
- Cache store: the DynamoDB `ephemeral` table with a 6-hour TTL (no separate cache cluster, no VPC). Level 3: LLM explanation text cached by `hash(MatchResult)`.
- Re-matching triggers: listing edit, proof added (ladder change), new vendor coverage in the cell, scheduled refresh for listings with no offers after N hours (EventBridge Scheduler).

### 12.8 Kabad Rate Index
Nightly job (Step Functions → Athena query over the DynamoDB export of settled deal lines): median, IQR and count per `(city, category, grade, week)`, written back to `RATE#…` items in `core`; published only above the k-anonymity threshold; outliers beyond a set IQR multiple excluded and reviewed by ops. Served through a cached public API (CloudFront in front of the public endpoints).

---

## 13. Settlement, payments and ledger

### 13.1 Principles
- **Final amount = Σ(agreed rate × weighed weight) ± agreed modifiers**, computed by `kabados-pricing` from the scale reading.
- **The seller never hands over goods unpaid:** the handover OTP is released only after payment capture (Mode A) or after the seller confirms receipt (Mode B).
- **KabadOS never holds funds in its own wallet.** Money moves through Razorpay as payment aggregator (AWS offers no payment gateway for Indian UPI); the platform issues no stored-value instruments.
- **Every movement is a double-entry ledger record**, reconciled daily against Razorpay settlement reports.

### 13.2 Payment modes
| Mode | When | Flow | Why |
|---|---|---|---|
| **B: Direct** (default) | Vendor self-pickup, small deals | Vendor pays seller directly (UPI QR/intent generated with the exact amount, or cash). Seller taps "received". The platform fee is billed to the vendor separately (prepaid lead credits or periodic invoice via Razorpay payment link). | Lowest regulatory and operational load; matches how kabadiwalas already pay. |
| **A: Platform-mediated** | Transporter pickups (vendor not present), larger deals, sellers who want platform protection | After weighing, the vendor gets a payment request (Razorpay Checkout, UPI) for **seller amount + transporter fare + platform fee**. On capture, **Route** transfers the fare to the transporter's linked account; the seller is paid via **RazorpayX Payout** to a validated UPI ID; the fee stays with the platform. Seller sees "paid" → shares handover OTP. | The seller is paid before loading even when the vendor is remote; the transporter is paid automatically. |

**Provider constraints:**
- Route linked accounts need KYC, and RBI's Payment Aggregator guidelines (Sep 2025) set eligibility criteria including minimum turnover ([Razorpay Route](https://razorpay.com/docs/payments/route/)). Mode A is therefore a per-environment configuration (AppConfig) enabled only when Route and RazorpayX are active on the account.
- RazorpayX **Fund Account Validation is supported on RazorpayX Lite, not on the Current Account** ([RazorpayX FAQ](https://razorpay.com/docs/x/fund-accounts/faqs)); RazorpayX Lite is used so seller UPI IDs are validated before payouts.
- Razorpay is called over its REST API from Rust (`reqwest`); webhooks are verified with HMAC-SHA256 (`hmac` + `sha2` crates).

### 13.3 Transporter payment
Vendor-chosen and vendor-paid. Mode A: automatic Route transfer on capture. Mode B: the vendor pays the transporter directly and marks it paid; the transporter confirms. The job record holds the agreed fare and any recorded changes.

### 13.4 Ledger
- Accounts per party and purpose: `seller_receivable`, `vendor_payable`, `transporter_payable`, `platform_fee_revenue`, `gst_output`, `gateway_fees`, `refunds`, `suspense`.
- Entries are immutable, idempotent (keyed by Razorpay event ID or internal command ID) and grouped into balanced transactions written with one DynamoDB `TransactWriteItems` call that also updates the affected account balances with version conditions (§16.6).
- **Reconciliation:** a daily Rust job matches captured payments, transfers, payouts and settlements; mismatches go to a finance queue.
- **Webhooks:** signature-verified and processed exactly once using idempotency items in the `ephemeral` table (conditional `PutItem` on the event ID, TTL for cleanup).
- **Why double-entry:** every rupee is accounted for on both sides, which makes reconciliation, audits and dispute refunds mechanical rather than investigative.

### 13.5 Disputes and cancellations
- **Weight dispute:** the seller disputes the reading → re-weigh on site with video, else ops reviews the evidence. The deal doesn't proceed to payment until resolved or the line is removed.
- **Item mismatch (re-grade):** the actual category differs from the claim → the line is re-priced with the vendor's rule for the actual category; the seller accepts the line, removes it, or cancels.
- **No-shows:** reason codes affect reliability scores; repeated no-shows can carry a fee under the published policy.
- **Refunds and reversals (Mode A):** Route reversals and payout-failure handling with automatic retries and ops alerts.

### 13.6 Tax and invoicing
- Platform fee invoices with GST, generated as PDFs by a Rust worker and stored in S3.
- Obligations to be confirmed by a chartered accountant before go-live: GST TCS for e-commerce operators (CGST s.52), income-tax TDS for e-commerce operators (s.194-O), and TCS on scrap sales (s.206C) where vendors are registered businesses. The ledger stores the fields any of these require.

---

## 14. Trust, safety and fraud

| Threat | Control |
|---|---|
| Photos copied from the internet | Camera capture for proof photos; gallery uploads labelled; perceptual-hash (`image_hasher` crate) duplicate detection across all listings; EXIF and capture-time consistency checks. |
| Photos reused across accounts | pHash near-duplicate search; device fingerprint and phone/UPI linkage graph. |
| Prompt injection in descriptions | §11.9: data-not-instructions, prompt-attack classifier, ladder moves only from images. |
| Weight tampering | Scale photo with the item on it; AI reading; seller confirmation; re-weigh-with-video dispute flow; vendor scale calibration photo during KYC. |
| Fake pickups / transporter fraud | Location Service geofences, handover OTP, Play Integrity verdicts, mock-location detection, photo timestamps vs. server time. |
| Identity fraud at onboarding | Textract extraction + KYC provider check against government records; Rekognition `CompareFaces` selfie vs. licence; ops review on mismatch. |
| Off-platform deals and scams | No phone numbers exchanged (in-app calls only); soft warnings on phone/UPI sharing in pre-deal chat; payment protection only for platform-paid deals. |
| Unauthorized hazardous handling | Cedar `forbid` policy on offers for hazardous lots unless the vendor's authorization is valid; expiry auto-locks categories. |
| Abuse in chat | Gemini safety settings + abuse classifier; report/block; ops review. |
| OTP pumping / account farming | AWS WAF rate rules and CAPTCHA, Cognito threat protection, per-number throttles in the sender trigger, messaging spend alarms. |
| Unsafe in-home pickups | Two-way ratings, verified IDs for vendors and transporters, share-trip-status link, in-app SOS that shares live job details with an emergency contact. |

**Scores** (Bayesian-smoothed so new users aren't penalized; appealable):
- **Seller honesty:** rolling accuracy of estimated vs. weighed weight, plus the share of claims that hold at pickup. Shown to vendors as a badge.
- **Vendor reliability:** on-time pickup, completion, payment speed, dispute rate.
- **Transporter reliability:** acceptance rate, on-time, evidence completeness.

---

## 15. System architecture and tech stack

### 15.1 High-level architecture
```
 ┌──────────────────────────── Clients ─────────────────────────────┐
 │ Seller web · Vendor web · Ops console · Public pages             │  Next.js static export on
 │   (client-rendered, Amplify JS Auth, TanStack Query, MapLibre)   │  AWS Amplify Hosting (CloudFront)
 │ Transporter app: Dioxus (Rust) + Kotlin bridge (FCM, CameraX,    │
 │   foreground service, Keystore, Chime SDK, Play Integrity)       │
 └───────────┬───────────────────────────┬──────────────────────────┘
             │ HTTPS + Cognito JWT       │ WebSocket (AppSync Events: chat, offers, job status)
   AWS WAF ──┤                           │
 ┌───────────▼───────────┐               │   Amazon Cognito (passwordless OTP)
 │ API Gateway HTTP API  │───────────────┘     └─ Custom SMS Sender (Rust) → End User Messaging Social (WhatsApp)
 │ (JWT authorizer)      │                                                 → End User Messaging SMS (DLT)
 └───────────┬───────────┘                    Amazon Verified Permissions (Cedar) ◄── authorization checks
 ┌───────────▼──────────────────────────────────────────────────────────────────┐
 │ Rust Lambdas (axum + lambda_http, arm64, provided.al2023), one per context:  │
 │ identity · listings · pricing · matching · offers · deals · logistics ·      │
 │ payments/ledger · chat · calls · drives · notifications · ops · public       │
 └──┬──────────────┬─────────────┬───────────────┬───────────────┬──────────────┘
    │              │             │               │               │
 DynamoDB         DynamoDB      S3 (images,    EventBridge ──► SQS ──► Rust workers & AI agents
 (core, ledger,   Streams ──►   receipts,       domain events         (Gemini API via                
 chat, ephemeral, outbox/events invoices)                              Rekognition, Textract)
 audit)                         CloudFront     Step Functions (deal/job lifecycles)   EventBridge Scheduler
 Amazon Location Service (Places, Maps, Routes, Trackers, Geofences)    Amazon Chime SDK (in-app calls)
 SNS mobile push → FCM (Android) · SES (email) · AppConfig (flags) · Secrets Manager · KMS
 DynamoDB export + Firehose → S3 data lake → Glue/Athena → QuickSight (ops analytics, Rate Index job)
 CloudWatch Logs/Metrics/RUM · X-Ray (OpenTelemetry)
 Third parties: Gemini API (AI) · Razorpay (payments) · KYC provider (government records) · Google Play / Play Integrity / FCM
```

### 15.2 Technology choices
| Layer | Choice | Why |
|---|---|---|
| **Backend language** | Rust (stable), Tokio | Fast cold starts and low memory on Lambda (lower cost), memory safety, strong types for money and state machines. |
| Lambda runtime | AWS Lambda Rust Runtime (`lambda_runtime`, `lambda_http`) on `provided.al2023`, arm64 (Graviton), built with **cargo-lambda** | Official AWS runtime crates; Graviton is cheaper per ms; cargo-lambda builds, tests and packages Rust functions. |
| HTTP framework | `axum` served through `lambda_http` | Idiomatic routing, extractors and tower middleware while running natively on Lambda. |
| Validation & schemas | `serde`, `garde` (validation), `schemars` (JSON Schema), `utoipa` (OpenAPI 3.1) | One set of Rust types defines wire format, validation, JSON Schemas for AI tools and the published API spec. |
| Database access | `aws-sdk-dynamodb` + `serde_dynamo`, typed repositories and key builders in `crates/db` | Item shapes and key formats defined once in Rust; no hand-built attribute maps. |
| AWS access | AWS SDK for Rust (`aws-sdk-*`: rekognition, textract, location, geoplaces, sfn, sqs, eventbridge, dynamodb, s3, socialmessaging, pinpointsmsvoicev2, chimesdkmeetings, verifiedpermissions, appconfigdata…) | First-party SDK for every AWS service used. |
| AI | Google Gemini API via the `gemini-client-api` crate (reference: `gemini-client-api/llms-client/gemini`), wrapped by `kabados-agent` | Existing paid Gemini plan; native Rust; JSON mode, function calling, multimodal and caching built in (§11.2). |
| Authorization | Amazon Verified Permissions (Cedar) + `cedar-policy` crate for CI policy tests | Managed policy store and decisions; the same Cedar engine (written in Rust) validates and tests policies in CI. |
| **Websites** | Next.js (App Router) with `output: 'export'`, React, TypeScript, Tailwind, shadcn/ui, TanStack Query, Serwist (PWA) | Static HTML/JS only, no server functions; client-side data from the Rust API; installable PWAs. |
| Website API client | TypeScript client generated from the backend's OpenAPI (`openapi-typescript` + `openapi-fetch`) | Types follow the Rust API automatically; no hand-written DTOs. |
| Website auth | AWS Amplify JS (Auth module only) | AWS-maintained client for Cognito passwordless flows. |
| Website maps | MapLibre GL JS with Amazon Location Maps styles | AWS map data rendered with an open-source renderer. |
| Hosting (web) | AWS Amplify Hosting (static) | CDN delivery via CloudFront, rewrite rules for deep links, preview deployments per branch. |
| **Transporter app** | Dioxus 0.7 (mobile, Android) + `dx` CLI; Kotlin bridge via `jni` (§8.2) | Rust end to end and shared crates with the backend; native features isolated in a small bridge. |
| Database | Amazon DynamoDB (on-demand, single-table `core` + `ledger`, `chat`, `ephemeral`, `audit`; Streams; Global Tables) | Serverless, no VPC or connection pools, millisecond latency at any scale, Streams-based outbox, cross-region DR (§16). |
| Cache | DynamoDB `ephemeral` table (TTL) + in-memory caches in warm Lambdas | Match cache, rate limiting and idempotency without running a cache cluster. |
| Files | S3 + CloudFront signed URLs; image-processing Rust Lambda (`image` crate) for thumbnails, EXIF stripping and face blurring | Private by default; vendors only see processed copies. |
| Async | EventBridge (domain events), SQS + DLQs, Step Functions, EventBridge Scheduler | Decoupled contexts; retries, timers and schedules without custom infrastructure. |
| Real-time | AWS AppSync Events (WebSocket pub/sub), published from Rust via its HTTP publish API | Serverless channels for chat, offers and job status with Cognito auth. |
| Identity & messaging | Cognito + End User Messaging Social (WhatsApp) + End User Messaging SMS (§5) | AWS-managed OTP lifecycle and delivery. |
| Maps & location | Amazon Location Service (Places v2, Maps, Routes, Trackers, Geofences) | Address search, distance/ETA, tracking and geofences on AWS. |
| Calls | Amazon Chime SDK (meetings API in ap-south-1; JS client on web, Android client in the bridge) | Audio calls without exchanging phone numbers (§19.3). |
| Push | Amazon SNS mobile push (FCM platform application) | AWS-side sending to Android devices. |
| Email | Amazon SES | Receipts, invoices and account email. |
| Config & secrets | AWS AppConfig, Secrets Manager, KMS | Runtime flags without deploys; managed secrets and encryption keys. |
| IaC | AWS CDK v2 (TypeScript) with the `cargo-lambda-cdk` `RustFunction` construct | Infrastructure as tested code; Rust functions built and packaged by CDK. |
| CI/CD | AWS CodePipeline + AWS CodeBuild (arm64 Linux images for Lambda; Android SDK/NDK image for Dioxus), GitHub source via AWS CodeConnections | AWS-native pipelines with IAM roles, no long-lived keys. |
| Local dev | `cargo lambda watch` (local Lambda emulation), DynamoDB Local and LocalStack in Docker Compose; Finch as container runtime option | Fast local loop without a shared cloud dev account. |
| Observability | `tracing` + `tracing-subscriber` (JSON logs to CloudWatch), CloudWatch embedded metric format, OpenTelemetry → X-Ray, CloudWatch RUM on websites | Structured logs, metrics and traces across API → workers → Gemini API; real-user monitoring for web. |

### 15.3 Repository layout
```
kabados/
├─ Cargo.toml                    # Rust workspace
├─ crates/
│  ├─ domain/                    # types, state machines, schemas (serde, schemars, utoipa)
│  ├─ pricing/                   # deterministic quote engine (no I/O; proptest)
│  ├─ agent/                     # agent runtime over gemini-client-api: budgets, validation, tracing
│  ├─ integrations/              # provider ports + mock / sandbox / live adapters (§15.4)
│  ├─ authz/                     # Verified Permissions client, entity loader, Cedar test harness
│  ├─ db/                        # DynamoDB key builders, item types, repositories
│  ├─ client/                    # Rust API client (used by the transporter app and evals)
│  ├─ copy/                      # generated UI copy for Rust clients
│  ├─ api-*/                     # one Lambda binary per bounded context (axum + lambda_http)
│  ├─ workers/                   # SQS/EventBridge/Step Functions task Lambdas
│  ├─ agents/                    # triage, itemize, verify, match, rules-author, scale-read Lambdas
│  └─ evals/                     # kabados-evals CLI
├─ policies/                     # Cedar schema + policies + allow/deny test tables
├─ copy/en.yaml                  # single source of UI copy → TS module + crates/copy
├─ web/                          # pnpm workspace
│  ├─ seller/                    # Next.js static export
│  ├─ vendor/                    # Next.js static export
│  ├─ ops/                       # Next.js static export
│  ├─ public/                    # Next.js static export (build-time data)
│  └─ packages/{ui,api-client,config}
├─ apps/transporter/             # Dioxus app (Rust)
│  └─ android-bridge/            # Kotlin module merged into the generated Gradle project
├─ infra/                        # AWS CDK app (stacks per context + shared)
├─ evals/                        # golden dataset manifests (data in S3)
└─ docs/                         # ADRs, runbooks, threat model, data inventory
```

### 15.4 External integrations: adapters, sandboxes and mocks
Every integration that needs an external approval or contract sits behind a Rust trait (a "port") in `crates/integrations` with three interchangeable implementations. Current provider status is tracked in [`external_api.md`](external_api.md).

| Mode | Behaviour | Used in |
|---|---|---|
| `mock` | In-process or DynamoDB-backed fake with realistic state transitions and delays; emits the **same internal events and webhook-shaped payloads** as the real provider | CI, local dev, demos, and any environment where the provider's approval isn't in place |
| `sandbox` | Provider's test mode (e.g., Razorpay test keys, WhatsApp test number) | dev, staging |
| `live` | Real provider | prod, staging smoke tests |

- **Ports:** `WhatsAppPort` (End User Messaging Social), `SmsPort` (End User Messaging SMS), `PaymentsPort`, `RoutePort`, `PayoutsPort` (Razorpay), `KycPort` (government-record verification provider), `IntegrityPort` (Play Integrity). AWS services that need no external approval (Location, Chime SDK, Textract, Rekognition) and the Gemini API (already paid) are called directly.
- **Why:** the whole product runs end to end regardless of which approvals are in place; switching a provider to live is a configuration change (AppConfig) plus a contract-test run. Mocks also act as fallbacks during provider outages (e.g., the KYC mock routes to manual ops review).
- **How:**
  - Mocks drive the real downstream code: a mock payment produces a signed `payment.captured`-shaped payload that goes through the **same** webhook receiver, ledger and deal workflow as production.
  - **Contract tests** run one suite against `mock` and `sandbox` so mocks can't drift from real behaviour.
  - A dev inbox (`/dev/inbox`, non-prod only) shows every mocked WhatsApp and SMS message, including OTPs.
  - **Honesty guardrails:** any user-facing flow backed by `mock` or `sandbox` shows a "Test mode — no real money or messages" banner; prod refuses to start with a money-moving or identity integration in `mock` unless a time-limited, alerted override is set; fixed OTP codes are allowed only for whitelisted demo numbers outside prod.

### 15.5 Language, input and UI copy
- All user-facing text (UI, AI outputs, notifications, WhatsApp/SMS templates) is **English**. All user input is typed text, photos and taps.
- UI copy has a single source (`copy/en.yaml`) compiled into a TypeScript module for the websites and a Rust module (`crates/copy`) for the transporter app.
  - **Why:** consistent wording across all clients, copy edits without touching components, and template text reviewable in one place (WhatsApp and DLT templates must match registered text exactly).
- Numbers, dates and currency use `en-IN` formatting (₹ and lakh-style digit grouping) on every client.

---

## 16. Data model (Amazon DynamoDB)

### 16.1 Approach
- **What:** Amazon DynamoDB is the only database. Data is modelled **access-pattern first** with a **single-table design** for the core domain, plus a few purpose-built tables where lifecycle or access rules differ. All tables use on-demand capacity, KMS customer-managed keys, point-in-time recovery, DynamoDB Streams and Global Tables replicas in ap-south-2.
- **Why:**
  - Fully serverless: no connection pools, no VPC for Lambdas (faster cold starts, no NAT cost), no instances to size; cost scales with traffic.
  - Single-digit-millisecond reads and writes at any scale for the key-based access patterns this marketplace has.
  - **Streams** give a transactional outbox for free: every state change emits an ordered change record that drives notifications, events and analytics.
  - **Global Tables** provide cross-region DR with seconds of replication lag instead of snapshot restores.
  - IAM fine-grained access (`dynamodb:LeadingKeys`) can limit each Lambda to its own key prefixes.
- **Goal:** p99 data-access latency < 20 ms for all online requests; zero hot partitions under peak load.
- **Trade-offs handled by design:**
  - No ad-hoc joins or SQL in the online path: every screen maps to a known key or index query (§16.4); ad-hoc analysis runs in Athena over S3 exports (§16.8).
  - No native geospatial queries: locations are keyed by **H3 cells**, so "near me" becomes "these N cells" (§12.5).
  - Multi-item invariants use `TransactWriteItems` (up to 100 items, all-or-nothing) and condition expressions; optimistic locking uses a `version` attribute.

### 16.2 Conventions
- Generic keys `PK`, `SK` and index keys `GSI1PK/GSI1SK`, `GSI2PK/GSI2SK`, `GSI3PK/GSI3SK`; every item has `type`, `schema_v`, `created_at`, `updated_at` and, when mutable, `version`.
- IDs are UUIDv7 (time-ordered, so `SK` ranges sort chronologically). Money in paise and weights in grams as integers; timestamps as ISO-8601 UTC strings.
- Sensitive attributes (phone, address text, landmark, document numbers) are field-level encrypted with KMS data keys before writing; lookups use keyed hashes.
- Rust access layer: `aws-sdk-dynamodb` + `serde_dynamo`, with all key builders and item types in `crates/db` so key formats exist in exactly one place.
- **Schema evolution:** readers accept the current and previous `schema_v`; backfills run as Step Functions Distributed Map jobs over S3 exports or parallel scans. No downtime migrations.

### 16.3 Tables
| Table | Purpose | Keys | Notes |
|---|---|---|---|
| `core` | Users, vendors, transporters, listings, offers, deals, jobs, drives, rule sets, coverage index, taxonomy, rate index | `PK`, `SK`; GSI1, GSI2, GSI3 | Single-table design; Streams → outbox/events/analytics |
| `ledger` | Double-entry ledger transactions, entries and account balances | `PK`, `SK` | Append-only by IAM policy (no `DeleteItem`/entry updates); Streams → S3 archive |
| `chat` | Chat messages | `PK=THREAD#<id>`, `SK=<ts>#<msgId>` | High write volume, separate scaling and retention |
| `ephemeral` | Idempotency keys, OTP channel preferences, match cache, rate-limit counters, dev inbox | `PK`, `SK` | Every item has a TTL attribute |
| `audit` | Append-only audit log of admin and security-relevant actions | `PK=SUBJECT#<id>`, `SK=<ts>#<actor>` | Streams → Firehose → S3 with Object Lock |

### 16.4 `core` item collections and access patterns
| Entity | PK | SK | Index keys | Serves |
|---|---|---|---|---|
| User profile | `USER#<id>` | `PROFILE` | GSI3: `COGNITO#<sub>` | Load profile; resolve user from token |
| Phone uniqueness | `PHONE#<hash>` | `PHONE` | — | One account per phone (written in the same transaction as the profile) |
| Role | `USER#<id>` | `ROLE#<role>` | — | Role checks, onboarding state |
| Address | `USER#<id>` | `ADDR#<id>` | — | Saved addresses (lat/lng, `h3_r7`, `h3_r8`, encrypted text) |
| Payout method | `USER#<id>` | `PAYOUT#<id>` | — | Validated UPI/bank accounts |
| Consent | `USER#<id>` | `CONSENT#<purpose>#<ver>` | — | DPDP consent records |
| Device | `USER#<id>` | `DEVICE#<deviceId>` | — | SNS endpoint ARN, integrity verdict |
| Vendor profile | `USER#<id>` | `VENDOR` | GSI1: `KYC#<status>` / `<submitted_at>` | Vendor details; ops KYC queue |
| Vendor document | `USER#<id>` | `DOC#<id>` | — | KYC and authorization documents with Textract output, expiry |
| Transporter profile | `USER#<id>` | `TRANSPORTER` | GSI1: `KYC#<status>` / `<submitted_at>` | Transporter details; KYC queue |
| Vehicle | `USER#<id>` | `VEHICLE#<id>` | — | Vehicle type, payload, RC |
| Vendor ↔ transporter | `USER#<vendorId>` | `MYTRANSPORTER#<transporterId>` | GSI2: `USER#<transporterId>` / `MYVENDOR#<vendorId>` | Trusted lists, both directions |
| Rule set version | `USER#<vendorId>` | `RULESET#<zero-padded version>` | — | Immutable versions; latest via reverse query |
| Coverage index | `COV#<h3_r8>#<categoryId>` | `VENDOR#<vendorId>` | — | Candidate vendors per cell and category (§12.5) |
| Listing | `LISTING#<id>` | `META` | GSI1: `FEED#<h3_r7>` / `<published_at>#<id>` (sparse: only while `PUBLISHED`); GSI2: `USER#<sellerId>` / `LISTING#<status>#<ts>` | Listing detail; vendor feed by area; seller's listings |
| Listing version | `LISTING#<id>` | `V#<ver>` | — | Description, asking price, claim reason, pickup preferences |
| Item / claim / proof request / image | `LISTING#<id>` | `V#<ver>#ITEM#<id>`, `…#CLAIM#<id>`, `…#PROOF#<id>`, `…#IMG#<id>` | — | Whole listing version in one `Query` |
| Match result | `LISTING#<id>` | `V#<ver>#MATCH` | — | Last computed match result (cache entries live in `ephemeral`) |
| Offer | `LISTING#<id>` | `OFFER#<offerId>` | GSI2: `USER#<vendorId>` / `OFFER#<status>#<ts>` | Offers on a listing (lines embedded); vendor's offers |
| Image hash band | `PHASH#<band>#<16-bit value>` | `IMG#<imageId>` | — | Near-duplicate candidate lookup (§14) |
| Deal | `DEAL#<id>` | `META` | GSI2: `USER#<sellerId>` and `USER#<vendorId>` (two index items) / `DEAL#<status>#<ts>` | Deal detail; "my deals" per party |
| Deal line / weighing | `DEAL#<id>` | `LINE#<n>`, `LINE#<n>#WEIGH#<ts>` | — | Final lines, scale readings |
| Job | `DEAL#<id>` | `JOB#<jobId>` | GSI2: `USER#<transporterId>` / `JOB#<status>#<ts>` | Transporter's jobs |
| Job event | `DEAL#<id>` | `JOB#<jobId>#EVT#<ts>` | — | Evidence timeline (GPS, photos, steps) |
| Call session / dispute / rating | `DEAL#<id>` | `CALL#<id>`, `DISPUTE#<id>`, `RATING#<fromUserId>` | GSI1 (disputes): `DISPUTE#<status>` / `<sla_due_at>` | Deal timeline; ops dispute queue by SLA |
| Drive | `DRIVE#<id>` | `META`, `MEMBER#<listingId>` | GSI2: `USER#<adminId>` / `DRIVE#<date>` | Drive detail and members |
| Category | `TAXONOMY` | `CAT#<categoryId>` | — | Taxonomy (small; cached in Lambda memory) |
| Authorized recycler | `RECYCLER#<state>` | `AUTH#<authorizationNo>` | — | Hazardous-category verification |
| Rate index | `RATE#<city>#<categoryId>#<grade>` | `WEEK#<isoWeek>` | — | Public price index |
| Notification outbox | `DEAL#<id>` / `LISTING#<id>` / `USER#<id>` | `NOTIFY#<ts>#<id>` | — | Written in the same transaction as the state change; consumed from Streams |

### 16.5 Consistency rules
- **Accept offer:** one `TransactWriteItems` sets the listing `META` to `RESERVED` (condition: status is `PUBLISHED` and `version` matches), marks the offer `ACCEPTED`, creates the deal `META` and its GSI2 index items, and writes the outbox item. Other offers are marked `SUPERSEDED` asynchronously by a Streams consumer, so the transaction stays small however many offers exist.
- **State transitions** (listing, deal, job) are conditional updates on `status` and `version`, so concurrent requests can't skip or repeat a transition.
- **Uniqueness** (one account per phone, one rating per party per deal) uses dedicated items created with `attribute_not_exists(PK)`.
- **Ledger:** see §13.4.

### 16.6 `ledger` table
| Item | PK | SK | Notes |
|---|---|---|---|
| Transaction | `TXN#<idempotencyKey>` | `META` | Kind, deal ID, occurred at; created with `attribute_not_exists` (exactly-once) |
| Entry | `TXN#<idempotencyKey>` | `ENTRY#<n>` | Account, `DEBIT`/`CREDIT`, amount (paise) |
| Account balance | `ACCT#<accountId>` | `BALANCE` | Running balance + `version`; updated in the same transaction as the entries |
| Account history | `ACCT#<accountId>` | `TXN#<ts>#<txnKey>` | Per-account statement lines |

### 16.7 `ephemeral` table (all items expire via TTL)
| Item | PK | SK | TTL |
|---|---|---|---|
| Idempotency key | `IDEM#<scope>#<key>` | `IDEM` | 24 h (API) / 7 days (webhooks) |
| OTP channel preference | `OTPCH#<phoneHash>` | `OTPCH` | Minutes |
| Match cache | `MATCH#<cacheKey>` | `MATCH` | 6 h (§12.7) |
| Rate-limit counter | `RL#<scope>#<subject>#<window>` | `RL` | Window length |
| Dev inbox message (non-prod) | `INBOX#<env>` | `<ts>#<id>` | 7 days |

### 16.8 Analytics and reporting
- **DynamoDB export to S3** (scheduled full and incremental exports) feeds the S3 data lake; Glue catalogs it and **Athena** runs the Rate Index job, ops reporting and ad-hoc analysis; QuickSight dashboards sit on top.
- **Streams → Kinesis Data Firehose → S3** for near-real-time product events.
- **Why:** analytical scans never touch the online tables, so reporting can't affect user-facing latency or cost.

---

## 17. API surface, events and real-time

### 17.1 Conventions
REST over HTTPS, JSON, `/v1` prefix, Cognito JWT bearer auth (API Gateway JWT authorizer), `Idempotency-Key` header required on every POST that creates or moves something, cursor pagination, RFC 9457 problem-details errors. **OpenAPI 3.1 is generated from the Rust handlers with `utoipa`** and published as (a) the TypeScript client for the websites and (b) the `kabados-client` crate for the transporter app.

### 17.2 Endpoints
**Auth helpers:** `POST /v1/auth/otp-channel` (set WhatsApp/SMS preference before a resend; unauthenticated, WAF-protected).

**Me and identity:** `GET/PATCH /v1/me`, `POST /v1/me/roles/{role}:apply`, `GET/POST/DELETE /v1/me/addresses`, `POST /v1/me/payout-methods` (triggers validation), `POST /v1/me/data-export`, `DELETE /v1/me` (erasure workflow).

**Places (proxy to Amazon Location):** `GET /v1/places/autocomplete?q&bias`, `GET /v1/places/geocode?q`, `GET /v1/places/reverse?lat&lng` (cached, rate-limited).

**Seller listings:**
- `POST /v1/listings` → draft
- `POST /v1/listings/{id}/images:presign` → presigned S3 PUT (content type, size and checksum enforced)
- `POST /v1/listings/{id}:analyze` → async; progress on AppSync channel `listing/{id}`
- `GET /v1/listings/{id}` · `PATCH /v1/listings/{id}/items` · `POST /v1/listings/{id}/claims/{claimId}:contest`
- `POST /v1/listings/{id}/proofs` (attach images to a proof request) · `POST /v1/listings/{id}/proofs:skip`
- `GET /v1/listings/{id}/price-suggestion`
- `POST /v1/listings/{id}:publish` · `POST /v1/listings/{id}:withdraw`
- `GET /v1/listings/{id}/offers` · `POST /v1/offers/{id}:accept` · `POST /v1/offers/{id}:counter` · `POST /v1/offers/{id}:reject`

**Vendor:**
- `GET /v1/vendor/feed?cells&categories&minValue&ladder&pooled&cursor` · `GET /v1/vendor/matches`
- `POST /v1/listings/{id}/offers` (server pre-fills from rules; client may edit lines)
- `GET/POST /v1/vendor/rule-sets` · `POST /v1/vendor/rule-sets:draft-from-text` (typed instruction) · `POST /v1/vendor/rule-sets/{id}/versions/{v}:simulate` · `POST .../{v}:publish`
- `GET /v1/vendor/transporters` · `POST /v1/vendor/transporters:invite` · `POST /v1/deals/{id}/jobs` (offer to one or broadcast)

**Transporter:** `PUT /v1/transporter/availability`, `PUT /v1/transporter/device` (SNS endpoint registration + integrity token), `GET /v1/transporter/jobs?status`, `POST /v1/jobs/{id}:accept|:decline`, `POST /v1/jobs/{id}/events` (typed: `ARRIVED`, `PHOTOS`, `LOADED`, `ARRIVED_YARD`...), `POST /v1/jobs/{id}/positions` (batched GPS → Location tracker).

**Deals:** `GET /v1/deals/{id}`, `POST /v1/deals/{id}/weighings`, `POST /v1/deals/{id}/lines/{lineId}:regrade|:remove`, `POST /v1/deals/{id}:confirm-weights` (seller), `POST /v1/deals/{id}/payment-request` (Mode A), `POST /v1/deals/{id}:mark-paid-direct` / `:confirm-received` (Mode B), `POST /v1/deals/{id}:handover` (OTP), `POST /v1/deals/{id}:cancel`, `POST /v1/deals/{id}/disputes`, `POST /v1/deals/{id}/ratings`, `GET /v1/deals/{id}/receipt`.

**Calls:** `POST /v1/deals/{id}/calls` (creates a Chime SDK meeting in ap-south-1 and attendee tokens for authorized deal participants), `POST /v1/calls/{id}:end`.

**Chat:** `POST /v1/threads`, `GET /v1/threads/{id}/messages`, `POST /v1/threads/{id}/messages` (server moderates, then publishes to channel `thread/{id}`).

**Drives:** `POST /v1/drives`, `GET /v1/drives/{id}`, `POST /v1/drives/{id}:join`, `GET /v1/drives/{id}/report`.

**Public:** `GET /v1/public/rate-index?city&category`, `GET /v1/public/impact?city`, `GET /v1/public/receipts/{signedId}`.

**Ops:** `/v1/ops/kyc/*`, `/v1/ops/ai-reviews/*`, `/v1/ops/disputes/*`, `/v1/ops/taxonomy/*`, `/v1/ops/fraud/*`, `/v1/ops/config/*` (staff user pool tokens with MFA only).

**Inbound webhooks:** `POST /webhooks/razorpay` (HMAC signature), `POST /webhooks/kyc-provider` (provider signature). A receiver Lambda verifies each request, stores the raw payload in S3 and emits an internal event. WhatsApp and SMS delivery events arrive from End User Messaging through SNS → SQS, not public webhooks.

### 17.3 Domain events (EventBridge, versioned schemas in the schema registry)
`ListingAnalyzed`, `ProofRequested`, `ProofProvided`, `ListingPublished`, `ListingVersionCreated`, `MatchComputed`, `VendorNotified`, `OfferMade`, `OfferAccepted`, `DealCreated`, `JobOffered`, `JobAccepted`, `JobTimedOut`, `ArrivedAtPickup`, `LineWeighed`, `LineRegraded`, `WeightsConfirmed`, `PaymentCaptured`, `PayoutProcessed`, `HandoverCompleted`, `DeliveredToYard`, `DealSettled`, `DisputeOpened`, `DisputeResolved`, `RuleSetPublished`, `VendorAuthorizationChanged`, `RatingSubmitted`. Location Service geofence events (`ENTER`/`EXIT`) are consumed from the default event bus and mapped to `ArrivedAtPickup` / `DeliveredToYard`.

Consumers: coverage-index builder, matching, notifications, ledger, impact aggregator, Rate Index job, AI ground-truth collector, analytics firehose.

### 17.4 Real-time channels (AppSync Events)
`listing/{id}` (analysis progress, new offers), `thread/{id}` (chat), `deal/{id}` (status, weights, payment), `vendor/{id}/matches` (first-look leads), `transporter/{id}/offers` (job offers while on duty), `job/{id}` (transporter status and location for seller and vendor). Channel subscriptions are authorized with the same Cedar policies as the REST resources (an AppSync Lambda authorizer written in Rust): a user can subscribe only if they could `GET` the resource.

---

## 18. Authorization with Cedar

- **Approach:** Amazon Verified Permissions with Cedar policies as the single source of truth for who can see and do what.
- **Why:** the most sensitive rules are relationship- and state-dependent ("this vendor may see this exact address only while their deal is active"; "only authorized vendors may offer on hazardous lots"; "only deal participants may start a call"). As Cedar policies they're explicit, reviewable, unit-testable and auditable, instead of `if` statements scattered across services. Cedar is an AWS open-source project written in Rust; Verified Permissions is its managed service.
- **Goal:** zero unauthorized address disclosures and zero hazardous offers from unauthorized vendors, provable from policy tests and decision logs.

### 18.1 Entities
`Seller`, `Vendor` (attrs: `kycApproved`, `hazardousAuthValid`, `suspended`), `Transporter` (attrs: `kycApproved`), `Listing` (attrs: `seller`, `status`, `containsHazardous`, `acceptedVendor?`, `dealStatus?`, `assignedTransporter?`, `jobStatus?`), `Deal` (attrs: `seller`, `vendor`, `transporter?`, `status`), `Offer`, `Thread`, `Drive`, `OpsAgent`.

Time-dependent facts (e.g., authorization expiry) are resolved into boolean attributes by the entity loader (`crates/authz`) at request time, which keeps policies simple and fast.

### 18.2 Policies (excerpt)
```cedar
// Sellers manage their own listings.
permit (principal is Seller, action in [Action::"ViewListing", Action::"EditListing", Action::"PublishListing"], resource is Listing)
when { resource.seller == principal };

// Any approved vendor can view a published listing (approximate location only).
permit (principal is Vendor, action == Action::"ViewListing", resource is Listing)
when { principal.kycApproved && !principal.suspended && resource.status == "PUBLISHED" };

// Exact address only for the vendor of an active deal.
permit (principal is Vendor, action == Action::"ViewExactAddress", resource is Listing)
when {
  resource has acceptedVendor && resource.acceptedVendor == principal &&
  resource has dealStatus &&
  ["CREATED", "PICKUP_SCHEDULED", "JOB_ASSIGNED", "AT_PICKUP", "WEIGHED",
   "SELLER_CONFIRMED", "AWAITING_PAYMENT", "PAID"].contains(resource.dealStatus)
};

// Transporter sees the address only while their job is active.
permit (principal is Transporter, action == Action::"ViewExactAddress", resource is Listing)
when {
  resource has assignedTransporter && resource.assignedTransporter == principal &&
  resource has jobStatus && ["ACCEPTED", "EN_ROUTE", "ARRIVED_PICKUP"].contains(resource.jobStatus)
};

// Vendors may offer on published listings.
permit (principal is Vendor, action == Action::"MakeOffer", resource is Listing)
when { principal.kycApproved && !principal.suspended && resource.status == "PUBLISHED" };

// Hazardous lots: only vendors with a valid authorization. Forbid always overrides permit.
forbid (principal is Vendor, action == Action::"MakeOffer", resource is Listing)
when { resource.containsHazardous && !principal.hazardousAuthValid };

// In-app calls only between participants of an open deal.
permit (principal, action == Action::"StartCall", resource is Deal)
when {
  (principal == resource.seller || principal == resource.vendor ||
   (resource has transporter && principal == resource.transporter)) &&
  !["SETTLED", "CLOSED", "CANCELLED"].contains(resource.status)
};
```

### 18.3 Engineering
- Policies and schema live in `policies/`, validated and tested in CI with the `cedar-policy` crate against allow/deny tables covering every role × action × deal state.
- Deployed to a Verified Permissions policy store per environment through CDK.
- Rust services call `IsAuthorized` / `BatchIsAuthorized` (feed pages check many listings in one call) via `aws-sdk-verifiedpermissions`; decisions cached per request.
- Feed queries also apply the same constraints when reading the feed index for efficiency; Cedar is the source of truth and is checked on detail views and on every mutating call.

---

## 19. Notifications and calls

### 19.1 Channels
| Event | Seller | Vendor | Transporter | Channel order |
|---|---|---|---|---|
| OTP | ✓ | ✓ | ✓ | WhatsApp (End User Messaging Social) → SMS (End User Messaging SMS) |
| Proof requested / analysis done | ✓ | | | In-app real-time → WhatsApp utility template |
| New match (first look) | | ✓ | | Web Push (PWA) → WhatsApp utility template |
| New offer / counter | ✓ | ✓ | | In-app → WhatsApp utility template |
| Job offer | | | ✓ | AppSync Events on the on-duty connection + **SNS → FCM high-priority** with full-screen intent |
| Pickup ETA / arrived | ✓ | ✓ | | In-app → WhatsApp utility template |
| Weights to confirm / payment received | ✓ | ✓ | | In-app → WhatsApp utility template → SMS |
| Receipts and invoices | ✓ | ✓ | | Email (SES) + WhatsApp document |

### 19.2 Delivery mechanics
- **Outbox pattern:** domain services write a notification item in the same `TransactWriteItems` call as the state change; a Rust Lambda consuming DynamoDB Streams sends it and records delivery.
  - **Why:** no lost or duplicated notifications when a Lambda retries.
- **Web Push** uses the standard Push API with VAPID keys (stored in Secrets Manager), sent from Rust (`web-push` crate).
- Templates are versioned; WhatsApp templates are approved through the WhatsApp Business Account linked to End User Messaging Social; SMS templates are DLT-registered (one Template ID per message type).
- Delivery and read events from End User Messaging flow through SNS → SQS into the notifications worker, which drives fallbacks (e.g., WhatsApp undelivered → SMS for payment-critical messages).
- User preferences and quiet hours apply, except to security and active-job messages.

### 19.3 In-app calls (Amazon Chime SDK)
- **What:** a "Call" button on active deals and jobs. The API checks the Cedar `StartCall` policy, creates a Chime SDK meeting with media in ap-south-1 and returns attendee tokens; the callee is notified (push/WebSocket) and joins. Web clients use the Chime SDK for JavaScript; the transporter app uses the Chime SDK for Android through the bridge.
- **Why:** parties can talk without ever learning each other's phone numbers, which protects privacy and safety and keeps deals on the platform; Chime SDK meeting control and media both run in Mumbai.
- **Details:** audio-only; meetings end when either party hangs up or after a maximum duration; `call_sessions` stores metadata only (no recordings).

---

## 20. Security, privacy and compliance

### 20.1 Security baseline
- AWS Organizations with separate accounts for dev, staging, prod and security/logging; SCPs; IAM Identity Center for staff; least-privilege IAM role per Lambda.
- AWS WAF on CloudFront and API Gateway: managed rule groups, rate-based rules, Bot Control and CAPTCHA/Challenge on auth endpoints.
- KMS encryption at rest everywhere; TLS 1.2+ in transit; field-level envelope encryption (KMS data keys) for phone numbers, addresses and document numbers; hashed phone numbers for lookups.
- DynamoDB access through IAM only (no database credentials); per-function policies restricted to the tables, indexes and key prefixes (`dynamodb:LeadingKeys`) each function needs; secrets for third parties in AWS Secrets Manager with rotation.
- S3: public access blocked; presigned uploads with size and type limits; malware scanning for document uploads (GuardDuty Malware Protection for S3).
- Security Hub, GuardDuty and IAM Access Analyzer enabled in every account.
- `cargo audit` / `cargo deny` and npm audit in CI; Amazon Inspector for Lambda code scanning; third-party penetration test before production traffic.
- Threat model maintained in `docs/threat-model.md` (STRIDE per bounded context).

### 20.2 Privacy: DPDP Act 2023 and Rules 2025
The DPDP Rules were notified on 13 Nov 2025 with phased commencement, most obligations applying from **13 May 2027** ([Hogan Lovells](https://www.hoganlovells.com/en/publications/indias-digital-personal-data-protection-act-2023-brought-into-force-)). The system implements them as follows:
- **Notice and consent:** itemized purposes (service delivery, AI improvement using photos, marketing) with separate, withdrawable consents recorded in `consents`.
- **Data minimization and progressive disclosure:** approximate location until a deal is accepted (Cedar-enforced); phone numbers never shown to other users (in-app calls); faces blurred; EXIF location stripped from vendor-visible images.
- **Data principal rights:** export, correction, erasure (Step Functions workflow across DynamoDB, S3 and Cognito, with legal-hold exceptions for financial records), grievance redressal with a published officer contact and SLA.
- **Retention:** S3 lifecycle rules delete or anonymize listing photos a set period after deal closure unless consented for AI improvement; financial records kept for the period tax law requires.
- **Breach response:** runbook covering notification to the Data Protection Board and affected users within the prescribed timelines (reported as 72 hours for the Board).
- **Children:** adults (18+) only.
- **AI processing outside AWS:** listing photos and descriptions are processed by the Google Gemini API, outside AWS and not guaranteed to stay in India. The DPDP Act permits cross-border transfer except to countries the government restricts; the privacy notice discloses Google as an AI processor, only minimized data is sent (redacted text; downscaled, EXIF-stripped, face-blurred images), and the paid-tier data-use terms of the Gemini API are reviewed and recorded before go-live.
- **Data residency:** primary data in ap-south-1 (Mumbai), disaster recovery in ap-south-2 (Hyderabad); End User Messaging Social, Chime SDK and Location resources in ap-south-1.

### 20.3 Environmental and sector compliance
- Hazardous categories are offered only to vendors with valid SPCB authorization (or a verified tie-up with an authorized recycler), checked by ops against CPCB/SPCB lists (imported as `RECYCLER#…` items), with expiry monitoring.
- Green Receipts state that they are platform records, not EPR certificates.
- Legal sign-off required before go-live on: marketplace terms, intermediary obligations under the IT Rules, payment flows, e-waste handling responsibilities, and tax (§13.6).

---

## 21. Infrastructure, environments and delivery

### 21.1 Environments
| Env | Account | Purpose | Data |
|---|---|---|---|
| local | none (`cargo lambda watch`, Docker, LocalStack) | Developer loop | Seed data |
| dev | dev account | Integration; preview stacks per branch (websites via Amplify branch deploys) | Synthetic |
| staging | staging account | Release candidates, load tests, AI evals | Synthetic + anonymized samples |
| prod | prod account (ap-south-1, DR ap-south-2) | Live | Real |

### 21.2 Infrastructure as code (CDK stacks)
`data` (DynamoDB tables with GSIs, Streams, PITR, Global Tables replicas, S3 export schedule; S3 buckets), `identity` (Cognito pools and triggers, Verified Permissions, End User Messaging configuration), `api` (API Gateway, Rust Lambdas via `RustFunction`, WAF), `async` (EventBridge, SQS, Step Functions, Scheduler), `ai` (agent Lambdas, Gemini API key secret, SQS queues), `realtime` (AppSync Events), `geo` (Location Service place index, map, route calculator, tracker, geofence collections), `comms` (SNS platform application, SES, Chime SDK permissions), `observability` (dashboards, alarms, RUM app monitors), `web` (Amplify apps), `analytics` (Firehose, Glue, Athena), `pipeline` (CodePipeline/CodeBuild).

### 21.3 CI/CD (AWS CodePipeline + CodeBuild)
- **Source:** GitHub via AWS CodeConnections; IAM roles per stage, no long-lived keys.
- **Backend pipeline:** `cargo fmt --check` → `cargo clippy -D warnings` → `cargo test` (unit, `proptest`, Cedar policy tests) → integration tests (DynamoDB Local and LocalStack via testcontainers) → AI eval suite (on prompt/model changes) → `cargo lambda build --release --arm64` → CDK diff/deploy to dev → E2E → manual approval → staging → canary to prod (Lambda aliases with CodeDeploy traffic shifting and automatic rollback on CloudWatch alarms).
- **Data changes:** table and index changes ship through CDK; item-shape changes follow the `schema_v` rules in §16.2, with backfills run as Step Functions Distributed Map jobs.
- **Websites:** `next build` (static export) → Amplify Hosting deploy per branch; public site rebuilt weekly by an EventBridge-triggered pipeline run.
- **Transporter app:** CodeBuild Android image → `dx bundle --platform android --release` → bridge module merged into the Gradle project → signed with the upload key from Secrets Manager → uploaded to a Google Play testing track via the Play Developer API → staged production rollout.

### 21.4 Resilience
- DynamoDB is multi-AZ by default, with point-in-time recovery and Global Tables replicas in ap-south-2 for regional failover; S3 versioning and cross-region replication for receipts and evidence.
- DLQs on every queue, with alarms and replay tooling.
- Targets: RPO ≤ 15 min, RTO ≤ 4 h for a regional failure.
- Graceful degradation: if the Gemini API is unavailable, listings publish without AI verdicts (flagged) and matching runs on the fast path only; if WhatsApp delivery fails, OTPs fall back to SMS.

---

## 22. Observability, SLOs, metrics and testing

### 22.1 SLOs
| Service | SLI | Target |
|---|---|---|
| Core API | Availability (non-5xx) | 99.9% monthly |
| Core API | p95 latency (reads / writes) | < 300 ms / < 600 ms |
| Photo analysis | p95 time from last upload to item list | < 25 s |
| Matching | p95 fast path / agent path | < 1.5 s / < 15 s |
| Job offer delivery | p95 time to device (on duty) | < 5 s |
| OTP delivery | p95 time from request to WhatsApp/SMS delivery | < 15 s |
| Payments | Webhook to deal update | < 10 s p95 |

### 22.2 Telemetry
- Structured JSON logs from `tracing` with correlation IDs across API → events → workers → Gemini API; CloudWatch Logs Insights queries saved per runbook.
- Metrics via CloudWatch embedded metric format; traces via OpenTelemetry exported to X-Ray (agent model calls and tool calls are spans with token counts).
- **CloudWatch RUM** on all websites (page performance, JS errors, API errors); the transporter app reports crashes and errors to a telemetry endpoint that writes to CloudWatch Logs.
- Alarms: error rates, DLQ depth, payment webhook failures, reconciliation mismatches, messaging spend spikes, Gemini API rate-limit and error spikes, hazard-recall evaluation regressions.

### 22.3 Product metrics (QuickSight dashboards over the S3 data lake)
Listing publish rate; time to first offer; offers per listing; offer→deal conversion; pickup on-time %; weight-estimate error; share of items at L2+; item-mismatch rate; dispute rate; payout time; hazardous kg routed to authorized vendors (must be 100%); kg diverted per month by category; vendor weekly active and retention; seller repeat rate; AI cost per listing.

### 22.4 Testing strategy
| Level | Tooling | Focus |
|---|---|---|
| Unit (Rust) | `cargo test` | Domain logic, state machines, validators, handlers |
| Property-based | `proptest` | Pricing invariants, rounding, schema round-trips |
| Policy | `cedar-policy` crate test tables | Every role × action × state combination |
| Integration | `testcontainers` (DynamoDB Local), LocalStack, `cargo lambda invoke` | Repositories, events, Lambda handlers, webhooks (Razorpay test-mode payloads) |
| Contract | OpenAPI diff checks; event schema registry checks; adapter contract suite (mock vs. sandbox) | Client/server, producer/consumer and mock/provider compatibility |
| Unit (web) | Vitest + Testing Library | Components, hooks, client-side flows |
| E2E web | Playwright | Seller listing → offer → deal; vendor rules → offer |
| E2E mobile | Maestro on Android emulators in CodeBuild | Job offer → checklist → handover |
| AI evals | `kabados-evals` over golden sets; model-graded only for proof-request wording, with human spot checks | Accuracy, hazard recall, regressions |
| Load | k6 | Feed, matching and offer bursts (morning peaks) |
| Security | OWASP ZAP baseline, `cargo audit`, Inspector, penetration test | OWASP ASVS L2 |
| Accessibility | axe in Playwright, manual screen-reader passes | WCAG 2.2 AA on websites |

---

## 23. Glossary
- **Kabad / kabadiwala:** scrap / scrap collector-dealer.
- **Ladder level:** verification level of a claim, L0–L4 (§11.5).
- **Rate card / rule set:** a vendor's pricing rules (§12.2).
- **Coverage index:** precomputed map of which vendors buy which categories in which H3 cells (§12.5).
- **H3:** hierarchical hexagonal geospatial index (open source).
- **Mode A / Mode B:** platform-mediated vs. direct payment (§13.2).
- **DLT:** TRAI's Distributed Ledger Technology registration required for commercial SMS in India.
- **SPCB / CPCB:** State / Central Pollution Control Board.
- **EPR:** Extended Producer Responsibility.
- **Green Receipt:** KabadOS record of a completed pickup with weights and chain of custody.
- **Native bridge:** the Kotlin module that gives the Dioxus app access to Android-only features (§8.2).
