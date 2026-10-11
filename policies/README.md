# KabadOS Cedar policies

Single source of truth for who can see and do what (PLAN.md §18).

| Path | What |
|---|---|
| `schema.cedarschema` | Cedar schema (entity types, attributes, actions). Edit this one. |
| `schema.cedarschema.json` | Same schema in canonical Cedar JSON, generated for Verified Permissions. Regenerate with `cargo run -p kabados-authz --example export_schema`; a test fails if it is stale. |
| `*.cedar` | One policy per file. The file stem (kebab-case) is the policy ID. Every file starts with a comment saying what the policy is for. |
| `tests/scenarios.json` | Readable allow/deny scenarios, run by `crates/authz/tests/scenarios.rs`. |

## Model

- Principals are typed by the role a user is acting in (`Seller`, `Vendor`, `Transporter`, `SocietyAdmin`, `OpsAgent`). One account can hold several roles, so the same user ID can appear as several types; a `Vendor` is never equal to a `Seller` entity.
- Statuses are the wire strings (`PUBLISHED`, `AT_PICKUP`, …).
- Time-dependent facts (KYC approval, hazardous-authorization expiry, suspension) are resolved into booleans by the entity loader in `crates/authz` at request time.
- `forbid` policies override every `permit`: hazardous lots for unauthorized vendors, offering on your own listing, and any action by a transporter without approved KYC or while suspended.

## Tests

`cargo test -p kabados-authz` runs:
- strict validation of every policy against the schema;
- exhaustive role × action × state tables (`crates/authz/tests/policy_tables.rs`), with expectations taken from the `kabados-domain` state machines where possible;
- the scenario table above.

`live_verified_permissions_round_trip` checks a deployed policy store and only runs with `KABADOS_AVP_POLICY_STORE_ID` set and `--ignored`.

## Deployment

The CDK identity stack creates one Verified Permissions policy store per environment, uploads `schema.cedarschema.json` as its schema (`ValidationMode: STRICT`), and creates one static policy per `*.cedar` file, using the file stem as the construct ID and the file contents as the statement. The Rust services call `IsAuthorized` / `BatchIsAuthorized` through `kabados_authz::VerifiedPermissions`, which sends the same entity JSON that the embedded engine (`LocalCedar`) evaluates in tests and local dev.
