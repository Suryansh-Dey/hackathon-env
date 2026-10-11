# KabadOS

A verified scrap marketplace for India. Sellers list scrap with photos; an AI
itemises it, checks claims and asks for proof; vendors' own pricing rules
produce instant quotes; vendors or their transporters pick up; every kilo is
weighed, paid for and traced. See [`PLAN.md`](PLAN.md) for the full design and
[`external_api.md`](external_api.md) for external services.

## Stack

- **Backend** — Rust on AWS Lambda (`axum` + `lambda_http`), DynamoDB single-table,
  EventBridge + SQS + Step Functions, Cedar authorization (Amazon Verified
  Permissions), Amazon Location / Rekognition / Textract / Chime SDK, Google
  Gemini for AI (via the `gemini-client-api` crate).
- **Websites** — Next.js static export (seller, vendor, ops, public), client-rendered
  against the Rust API, hosted on S3 + CloudFront.
- **Transporter app** — Dioxus (Rust) sharing the domain, pricing and client crates.
- **Infra** — AWS CDK v2 (TypeScript). **CI/CD** — GitHub Actions.

## Repository layout

```
crates/
  domain/        types, state machines, taxonomy, money (paise/grams), events
  pricing/       deterministic quote engine (pure, proptest)
  db/            DynamoDB repositories + in-memory twin (conformance-tested)
  authz/         Cedar schema, policies and local/Verified-Permissions clients
  runtime/       config, provider modes, secrets, KMS field encryption
  integrations/  provider ports (Razorpay, WhatsApp/SMS, Location, vision, …) with mock/live
  agent/ agents/ Gemini agent runtime and the seven agents (triage, itemize, verify, …)
  evals/         kabados-evals CLI + golden sets
  api/           REST API: axum routers per bounded context + the combined api-all Lambda
  workers/       Stream/SQS/EventBridge/scheduled worker Lambdas + the local dev server
  client/        typed Rust API client (transporter app)
policies/        Cedar policies (one per file) + allow/deny test tables
openapi/         generated OpenAPI 3.1 contract (source for the TS client)
web/             Next.js apps + shared api-client / ui / config packages
infra/           CDK app (data, async, compute, web, github-oidc stacks)
```

## Run it locally

One command runs the whole backend (API + in-process workers) on an in-memory
store — no AWS, no Docker:

```bash
cargo run -p kabados-workers --bin dev-server        # http://localhost:8080
```

Then, in another shell, load demo vendors with published rate cards and sign in:

```bash
curl -XPOST localhost:8080/dev/seed
curl -XPOST localhost:8080/dev/token -H 'content-type: application/json' -d '{"phone":"+919876543210"}'
# → { "token": "...", "sub": "..." }   use it as:  Authorization: Bearer <token>
```

Run a website against it:

```bash
cd web && npm install
NEXT_PUBLIC_API_URL=http://localhost:8080 npm run dev --workspace seller   # :3001
```

Sign-in is phone-number only in non-production (OTP is disabled); KYC is
auto-approved; Razorpay runs in test mode; Mode-A payments can be completed with
`POST /v1/payments/{orderId}:simulate-capture`.

## Test

```bash
cargo test --workspace                                   # unit, property, policy, pipeline
DYNAMODB_ENDPOINT=http://localhost:8000 \
  cargo test --workspace -- --ignored --skip live_       # DynamoDB Local integration
cargo run -p kabados-evals -- run --gate                 # AI eval gate (mock)
cd web && npm run typecheck && npm run build             # static-export the sites
cd infra && npm test                                     # CDK stack assertions
```

Live tests (named `live_*`) hit real Gemini / AWS and are skipped unless the
relevant credentials are set. CI (`.github/workflows/ci.yml`) runs the whole
suite on every push and pull request, plus an arm64 Lambda build.

## Deploy (AWS, `ap-south-1`)

```bash
cd infra && npm ci
npx cdk deploy kabados-dev-data kabados-dev-async        # tables, buckets, queues, bus
npx cdk deploy kabados-dev-compute                       # api-all + worker Lambdas + HTTP API
cd ../web && NEXT_PUBLIC_API_URL=<api-url> npm run build  # static-export the four apps
cd ../infra && npx cdk deploy kabados-dev-web            # S3 + CloudFront per app
```

Secrets live in AWS Secrets Manager under `/kabados/<env>/…` (Gemini key,
Razorpay test keys). GitHub Actions deploys via an OIDC role
(`kabados-dev-github-deploy`); no long-lived AWS keys.

**CI/CD** (`.github/workflows/ci.yml`): every push and PR runs the checks for
what it touched — Rust fmt/clippy/tests (+ DynamoDB Local) and the arm64 Lambda
build, infra tsc/vitest/`cdk synth`, the four web apps' static export and
typecheck, and the transporter app's fmt/clippy/tests. A green push to `main`
then runs the steps above automatically (role and region come from the repo
variables `AWS_DEPLOY_ROLE_ARN` / `AWS_REGION`) and smoke-tests
`/v1/public/config` and the seller sign-in page.

## Provider modes

Each integration runs in `mock`, `sandbox` or `live` mode (AWS AppConfig / env).
For this build: AI is **live** (Gemini), payments are **Razorpay test mode**,
and WhatsApp/SMS/KYC/Route/Payouts/Integrity are **mocked** — so the product
runs end to end regardless of which external approvals are in place. Production
refuses to start with a money-moving, identity or AI integration mocked.
