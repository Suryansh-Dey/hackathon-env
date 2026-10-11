# KabadOS — External APIs and Services to Set Up

Companion to [`PLAN.md`](PLAN.md). Lists every service that needs an account, approval, registration, key or contract before use: AWS services with account-level setup first (the platform is AWS-first), then third parties.

For each: what it's for, what to set up, **availability today**, and the **mock** used until approval comes through.

> **Rule:** every integration that needs an external approval sits behind a Rust port with `mock`, `sandbox` and `live` implementations (PLAN.md §15.4). Code paths, events and webhooks behave the same in all three modes, so switching to live is a configuration change, not a rewrite.

---

## 0. Availability legend and summary

| Status | Meaning |
|---|---|
| 🟢 **Now** | Usable immediately after enabling or with an existing account (no external approval). |
| 🟡 **Sandbox now, live after approval** | A test mode exists; live use needs verification, registration or activation. |
| 🔴 **Mock until approved** | Approval is a hard prerequisite (regulatory registration, commercial onboarding). Use our mock. |

### AWS services needing setup or approval
| # | Service | Used for | Status | Mock until live? |
|---|---|---|---|---|
| 1 | AWS End User Messaging Social (WhatsApp) | OTP via WhatsApp authentication templates; utility notifications | 🟡 (Meta business verification + template approval) | Mock in CI/demo; test number in dev |
| 2 | AWS End User Messaging SMS | OTP fallback, payment-critical SMS | 🔴 (needs TRAI DLT registration #13 + India sender ID) | Yes: SMS mock |
| 3 | Amazon Cognito | Passwordless OTP sign-in, Custom SMS Sender trigger, staff pool with MFA | 🟢 (delivery depends on #1–2) | No |
| 4 | Amazon SES | Receipts, invoices, account email | 🟡 (production access request) | Dev inbox in non-prod |
| 5 | Amazon SNS mobile push | Job-offer pushes to Android via FCM | 🟢 (needs Firebase project #15) | No |
| 6 | Amazon Location Service | Places (autocomplete, geocode), Maps, Routes, Trackers, Geofences | 🟢 | No |
| 7 | Amazon Chime SDK | In-app audio calls (no phone numbers exchanged) | 🟢 | No |
| 8 | Amazon Textract, Amazon Rekognition | KYC document extraction, label text, selfie match, moderation, face blurring | 🟢 | No |
| 9 | Core platform (DynamoDB, S3, Lambda, API Gateway, Verified Permissions, AppSync Events, Step Functions, EventBridge, AppConfig, Secrets Manager, KMS, WAF, CloudWatch/RUM, X-Ray, Firehose, Glue, Athena, QuickSight, Amplify Hosting, CodePipeline/CodeBuild/CodeConnections, Organizations, IAM Identity Center, GuardDuty, Security Hub, Inspector) | Platform | 🟢 | No |
| 10 | Service quotas | Lambda concurrency, messaging spend, SES rate, Step Functions, AppSync connections | Request before production traffic | — |

### Third parties
| # | Service | Used for | Status | Mock until live? |
|---|---|---|---|---|
| 11 | **Google Gemini API** (via `gemini-client-api` crate) | All AI: photo triage, itemization, claim verification, matching agent, rules authoring, scale reading, abuse classifier | 🟢 (paid plan already owned) | No (mock model client for CI only) |
| 12 | Google AI Studio / Google Cloud billing for the Gemini key | Key management, quotas, billing alerts | 🟢 | — |
| 13 | TRAI DLT registration | Prerequisite for any commercial SMS in India | 🔴 | Yes: SMS mock |
| 14 | Razorpay (Payments, Route, RazorpayX Payouts + FAV) | Deal payments, transporter split, seller payouts | 🟡 Payments (test keys now) / 🔴 Route & Payouts | Test mode; mocks for Route and Payouts |
| 15 | Firebase Cloud Messaging (FCM HTTP v1) | Android push transport used by SNS | 🟢 | No |
| 16 | Google Play Console + Play Integrity API | Distribute the transporter app; device integrity | 🟡 (developer account; integrity needs the app on Play) | Mock integrity verdict in dev |
| 17 | KYC provider (PAN, GSTIN, DL, RC; DigiLocker) | Verify Textract-extracted values against government records | 🔴 (commercial onboarding) | Yes: KYC mock + manual ops review |
| 18 | CPCB / SPCB authorized recycler lists | Verifying hazardous-category vendors | 🟢 data (published lists; no API) | Manual import |
| 19 | Emission factors dataset (e.g., US EPA WARM) | Impact estimates | 🟢 data | Static table |
| 20 | GitHub | Source repository (connected to CodePipeline via CodeConnections) | 🟢 | No |

**Start these first (longest lead times):** DLT registration (#13) and India sender ID (#2), WhatsApp Business Account verification and templates (#1), Razorpay activation + Route/RazorpayX (#14), KYC provider contract (#17), SES production access (#4). These can take days to weeks depending on the provider.

---

## 1. AWS End User Messaging Social (WhatsApp)

- **Why:** delivers OTPs (authentication templates) and utility notifications over WhatsApp from AWS; cheaper and more reliable than SMS in India, billed through AWS, available in ap-south-1 ([AWS docs](https://docs.aws.amazon.com/social-messaging/latest/userguide/what-is-service.html)).
- **Set up:**
  1. Meta Business account with **business verification**.
  2. In End User Messaging Social (ap-south-1), link a WhatsApp Business Account through Meta's embedded sign-up; register the phone number and get the display name approved.
  3. Create English **message templates**: *authentication* (OTP with copy-code button) and *utility* (offer received, pickup ETA, weights to confirm, payment received, receipt document).
  4. Configure an **event destination** (SNS topic) for delivery/read status, with an SQS subscription for the notifications worker.
  5. IAM permission for the Custom SMS Sender Lambda and notifications worker to send messages.
- **Config:** `WHATSAPP_MODE=mock|sandbox|live`; phone number ID and template names in AppConfig.
- **Before approval:** Meta's test business number in dev; **mock** (dev inbox) in CI and demos.

## 2. AWS End User Messaging SMS

- **Why:** SMS fallback for OTPs and payment-critical messages.
- **Set up:**
  1. Complete DLT registration (#13): Principal Entity ID, header (sender ID), Template IDs.
  2. Register the India sender ID ([guide](https://docs.aws.amazon.com/sms-voice/latest/userguide/registrations-sms-senderid-india.html)); request production access and a spend limit.
  3. Send with `EntityId` and `TemplateId` on every message; for Cognito-originated SMS set `InEntityId`/`InTemplateId` ([Cognito API](https://docs.aws.amazon.com/cognito-user-identity-pools/latest/APIReference/API_EumsSmsConfigurationType.html)).
  4. CloudWatch spend alarms (SMS-pumping protection).
- **Config:** `SMS_MODE=mock|live`; DLT IDs in Secrets Manager `/kabados/{env}/sms/dlt`.
- **Mock:** messages go to the dev inbox (`/dev/inbox`, non-prod only); fixed OTPs only for whitelisted demo numbers, **never** in prod (enforced at startup).

## 3. Amazon Cognito

- Customer user pool: passwordless sign-in (`ALLOW_USER_AUTH`, SMS OTP), **Custom SMS Sender trigger** (Rust Lambda) with a customer-managed KMS key, threat protection enabled.
- Staff user pool for the ops console with mandatory MFA.
- Public app clients (no secret) for websites and the transporter app; API Gateway JWT authorizer per pool.
- **Status:** 🟢; OTP delivery depends on #1 and #2.

## 4. Amazon SES

- Verify the sending domain (DKIM, SPF, DMARC in Route 53), request **production access**, configuration set with bounce/complaint handling via SNS.
- **Status:** 🟡; non-prod sends go to the dev inbox.

## 5. Amazon SNS mobile push

- Create an SNS **platform application** of type FCM with the Firebase service-account credentials (#15); the app registers its FCM token via `PUT /v1/transporter/device` and the backend creates an SNS endpoint per device.
- **Status:** 🟢.

## 6. Amazon Location Service

- In ap-south-1: Places v2 (`Autocomplete`, `Geocode`, `ReverseGeocode`), a map style for MapLibre GL JS, a route calculator, a **tracker** for transporter positions, **geofence collections** (pickup points, vendor yards) with EventBridge integration. Review data-provider terms for storing geocoding results.
- **Status:** 🟢.

## 7. Amazon Chime SDK

- IAM permissions for the calls Lambda (`CreateMeeting`, `CreateAttendee`, `DeleteMeeting`) with control and media region ap-south-1 ([regions](https://docs.aws.amazon.com/chime/latest/dg/sdk-available-regions.html)); Chime SDK for JavaScript on the websites; Chime SDK for Android in the transporter app's native bridge.
- **Status:** 🟢.

## 8. Amazon Textract and Amazon Rekognition

- Textract `AnalyzeDocument` (Queries) for PAN/GSTIN/DL/RC/authorization documents and `DetectDocumentText` for appliance labels; Rekognition `DetectModerationLabels`, `DetectFaces` (blurring before any photo leaves AWS) and `CompareFaces` (selfie vs. licence). Confirm feature availability in ap-south-1 at setup.
- **Status:** 🟢.

## 9. Core platform services

DynamoDB (on-demand tables with Streams, PITR, Global Tables replicas in ap-south-2, scheduled exports to S3), S3, Lambda (Rust, arm64), API Gateway HTTP API, Verified Permissions, AppSync Events (Cognito auth + Rust Lambda authorizer), Step Functions, EventBridge + Scheduler, AppConfig, Secrets Manager, KMS, WAF (rate rules, Bot Control, CAPTCHA), CloudWatch (Logs, metrics, RUM), X-Ray, Firehose, Glue, Athena, QuickSight, Amplify Hosting, CodePipeline, CodeBuild, CodeConnections, Organizations, IAM Identity Center, GuardDuty (incl. Malware Protection for S3), Security Hub, Inspector. All 🟢; provisioned through CDK.

## 10. Service quotas

Request before production traffic: Lambda concurrency, End User Messaging SMS spend limit and throughput, SES sending rate, Step Functions executions, AppSync Events connections, DynamoDB account-level throughput limits.

---

## 11. Google Gemini API (via `gemini-client-api`)

- **Why:** all AI in the platform (PLAN.md §11). The team already holds a paid Gemini API plan; the `gemini-client-api` crate is native async Rust and supports multimodal input, JSON mode with schemas from Rust types (`#[gemini_schema]`), function calling (`#[gemini_function]`, `execute_function_calls!`), thinking mode, context caching and streaming.
- **API reference:** library source at `gemini-client-api/llms-client/gemini` in this repo (README, `examples/`, `src/gemini/`).
- **Set up:**
  1. Use the existing paid API key; create separate keys per environment (dev, staging, prod).
  2. Store each key in Secrets Manager at `/kabados/{env}/gemini/api_key`; Lambdas read it once per container; never ship it to clients.
  3. Confirm rate limits (requests and tokens per minute) for the chosen Flash-class and Pro-class models against expected peak load; request increases if needed.
  4. Record the model IDs per agent in AppConfig (`ai/models`), so models change without deploys.
  5. Review and record the paid-tier **data-use terms** (training/retention of prompts and images) before go-live, as required by PLAN.md §20.2.
- **Config:** `AI_MODE=mock|live` (mock = deterministic fixture responses for CI and offline tests only).
- **Status:** 🟢.

## 12. Google AI Studio / Google Cloud billing

- Manage Gemini API keys, view usage, set **budget alerts** on the billing account, and restrict keys (API restrictions) to the Gemini API only.
- **Status:** 🟢.

## 13. TRAI DLT registration (SMS prerequisite)

- **Why:** TRAI requires every business sending commercial SMS in India to register its entity, sender IDs and templates; unregistered SMS is blocked.
- **Set up:** register the legal entity on an operator DLT portal (Jio, Airtel, Vi, BSNL) to get the **Principal Entity ID**; register **headers** (e.g., `KABDOS`); register a **content template** per SMS to get **Template IDs**.
- **Outputs:** PEID, headers, template IDs → Secrets Manager `/kabados/{env}/sms/dlt`.
- **Mock until done:** yes (#2).

## 14. Razorpay (Payments, Route, RazorpayX Payouts + FAV)

- **Why:** UPI-first payments in India: vendor payments (Mode A), platform fee collection, transporter fare split, seller payouts.
- **Payments:** test keys immediately; KYC for **activation**; webhook `/webhooks/razorpay` (events `payment.captured`, `payment.failed`, `order.paid`, `refund.processed`, plus Route and Payout events once enabled).
- **Route:** request activation; confirm eligibility under RBI Payment Aggregator guidelines (Sep 2025), including minimum turnover ([Razorpay Route](https://razorpay.com/docs/payments/route/)); linked accounts per transporter with KYC; transfers, settlement holds, reversals.
- **RazorpayX Payouts + FAV:** onboard to **RazorpayX Lite** (Fund Account Validation is supported on Lite, not the Current Account — [FAQ](https://razorpay.com/docs/x/fund-accounts/faqs)); `vpa` fund accounts validated with `POST /v1/fund_accounts/validations`; payouts with idempotency keys; payout webhooks; low-balance alerts.
- **Config:** `PAYMENTS_MODE`, `ROUTE_MODE`, `PAYOUTS_MODE` ∈ `{mock, sandbox, live}`; keys in `/kabados/{env}/razorpay/*` (test and live keys never share an environment).
- **Mocks:** Route mock records linked accounts and transfers with activation states; Payouts mock validates VPA format and moves payouts `queued → processing → processed` with a configurable failure rate. Both emit the same internal events as live webhooks; UI shows the "Test mode" banner.

## 15. Firebase Cloud Messaging (FCM HTTP v1)

- **Why:** Android delivers wake-up pushes only through FCM; SNS uses it as transport.
- **Set up:** Firebase project → add the Android app → service account with FCM permissions → JSON key in Secrets Manager → SNS platform application (#5). The Kotlin bridge includes the Firebase Messaging library and service.
- **Status:** 🟢.

## 16. Google Play Console + Play Integrity API

- **Set up:** developer account (one-time fee) → create the app → upload key in Secrets Manager (CodeBuild signs releases) → Play Developer API service account for automated uploads → enable the Play Integrity API in a linked Google Cloud project → verify integrity tokens server-side in Rust.
- **Mock:** dev builds return a configurable verdict (`MEETS_DEVICE_INTEGRITY` by default).

## 17. KYC provider (government-record verification)

- **Why:** Textract extracts document numbers and names, but checking them against government records (PAN, GSTIN, DL via Sarathi, RC via Vahan) needs a licensed verification provider.
- **Pick one** (compare price, coverage, SLA): e.g., IDfy, Signzy, HyperVerge, Surepass; DigiLocker pulls through the provider or via API Setu onboarding.
- **Set up:** commercial agreement → sandbox credentials → production credentials after compliance review; webhook `/webhooks/kyc-provider`.
- **Config:** `KYC_MODE=mock|sandbox|live`; credentials in `/kabados/{env}/kyc/*`.
- **Mock:** format checks only (PAN regex, GSTIN checksum, DL/RC patterns), returns `PENDING_MANUAL_REVIEW` and routes to the ops KYC queue; also the fallback when the provider is down.

## 18. CPCB / SPCB authorized recycler lists (data, no API)

- Download published lists from CPCB's EPR portal pages and SPCB websites; import as `RECYCLER#…` items in DynamoDB with source URL and import date; re-check monthly and on each vendor application. Dev uses clearly fake seed entries, never shipped to prod.

## 19. Emission factors dataset (data)

- Import per-material factors from a cited source (e.g., US EPA WARM, or India-specific factors where published); version the table; show source and version on every impact figure.

## 20. GitHub

- Source repository with branch protection and required checks; connected to CodePipeline via **AWS CodeConnections** (no long-lived AWS keys).

---

## 21. Configuration and secrets conventions

- **Provider modes** in AWS AppConfig per environment: `whatsapp`, `sms`, `payments`, `route`, `payouts`, `kyc`, `integrity`, `ai` ∈ `{mock, sandbox, live}`.
- **Startup guard:** prod refuses to start if any money-moving, identity or AI integration is in `mock`, unless a time-limited override flag is set and alerted on.
- **Secrets:** AWS Secrets Manager under `/kabados/{env}/{provider}/...` (`gemini/api_key`, `razorpay/*`, `sms/dlt`, `kyc/*`, `fcm/service_account`, `play/*`), rotated where the provider supports it, cached in memory by Rust services.
- **Webhooks:** only `/webhooks/razorpay` and `/webhooks/kyc-provider` are public; each verifies signatures and stores the raw payload in S3 before processing. AWS-originated events (messaging delivery, geofences) arrive through SNS/EventBridge.
- **Test-mode transparency:** any user-facing flow backed by a mock or sandbox shows a visible "Test mode" banner.
