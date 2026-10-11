//! Runs the readable scenario table in `policies/tests/scenarios.json`.

use kabados_authz::*;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::HashMap;

const SCENARIOS: &str = include_str!("../../../policies/tests/scenarios.json");

#[derive(Default)]
struct Users(HashMap<String, UserId>);

impl Users {
    fn id(&mut self, v: &Value) -> UserId {
        let name = v.as_str().expect("user name");
        self.0.entry(name.to_owned()).or_insert_with(UserId::new).clone()
    }

    fn opt(&mut self, v: Option<&Value>) -> Option<UserId> {
        v.map(|v| self.id(v))
    }
}

fn status<T: DeserializeOwned>(v: &Value) -> T {
    serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("bad status {v}: {e}"))
}

fn flag(spec: &Value, key: &str, default: bool) -> bool {
    spec.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn principal(spec: &Value, u: &mut Users) -> Principal {
    match spec["type"].as_str().unwrap() {
        "Seller" => Principal::Seller(u.id(&spec["user"])),
        "Vendor" => Principal::Vendor(VendorAttrs {
            id: u.id(&spec["user"]),
            kyc_approved: flag(spec, "kycApproved", true),
            hazardous_auth_valid: flag(spec, "hazardousAuthValid", false),
            suspended: flag(spec, "suspended", false),
        }),
        "Transporter" => Principal::Transporter(TransporterAttrs {
            id: u.id(&spec["user"]),
            kyc_approved: flag(spec, "kycApproved", true),
            suspended: flag(spec, "suspended", false),
        }),
        "SocietyAdmin" => Principal::SocietyAdmin(u.id(&spec["user"])),
        "Ops" => Principal::Ops { id: spec["user"].as_str().unwrap().to_owned(), mfa: flag(spec, "mfa", true) },
        other => panic!("unknown principal type {other}"),
    }
}

fn listing(spec: &Value, u: &mut Users) -> ListingAttrs {
    ListingAttrs {
        id: ListingId::new(),
        seller: u.id(&spec["seller"]),
        status: status(&spec["status"]),
        contains_hazardous: flag(spec, "containsHazardous", false),
        accepted_vendor: u.opt(spec.get("acceptedVendor")),
        deal_status: spec.get("dealStatus").map(status),
        assigned_transporter: u.opt(spec.get("assignedTransporter")),
        job_status: spec.get("jobStatus").map(status),
        drive_admin: u.opt(spec.get("driveAdmin")),
    }
}

fn deal(spec: &Value, u: &mut Users) -> DealAttrs {
    DealAttrs {
        id: DealId::new(),
        seller: u.id(&spec["seller"]),
        vendor: u.id(&spec["vendor"]),
        transporter: u.opt(spec.get("transporter")),
        status: status(&spec["status"]),
    }
}

fn resource(spec: &Value, u: &mut Users) -> Resource {
    match spec["type"].as_str().unwrap() {
        "Listing" => Resource::Listing(listing(spec, u)),
        "Deal" => Resource::Deal(deal(spec, u)),
        "Offer" => Resource::Offer(OfferAttrs {
            id: OfferId::new(),
            listing: listing(&spec["listing"], u),
            vendor: u.id(&spec["vendor"]),
            status: status(&spec["status"]),
        }),
        "Job" => Resource::Job(JobAttrs {
            id: JobId::new(),
            deal: deal(&spec["deal"], u),
            transporter: u.opt(spec.get("transporter")),
            offered_to: spec.get("offeredTo").and_then(Value::as_array).into_iter().flatten().map(|v| u.id(v)).collect(),
            status: status(&spec["status"]),
        }),
        "Thread" => Resource::Thread(ThreadAttrs {
            id: ThreadId::new(),
            seller: u.id(&spec["seller"]),
            vendor: u.id(&spec["vendor"]),
            listing_status: status(&spec["listingStatus"]),
            deal_open: flag(spec, "dealOpen", false),
        }),
        "Drive" => Resource::Drive(DriveAttrs { id: DriveId::new(), admin: u.id(&spec["admin"]), status: status(&spec["status"]) }),
        "Platform" => Resource::Platform,
        other => panic!("unknown resource type {other}"),
    }
}

#[test]
fn scenario_table() {
    let doc: Value = serde_json::from_str(SCENARIOS).expect("scenarios.json parses");
    let engine = LocalCedar::new().unwrap();
    let scenarios = doc["scenarios"].as_array().expect("scenarios array");
    assert!(scenarios.len() >= 20);
    let mut failures = Vec::new();
    for s in scenarios {
        let mut users = Users::default();
        let name = s["name"].as_str().unwrap();
        let p = principal(&s["principal"], &mut users);
        let a: Action = s["action"].as_str().unwrap().parse().unwrap();
        let r = resource(&s["resource"], &mut users);
        let want = match s["expect"].as_str().unwrap() {
            "ALLOW" => true,
            "DENY" => false,
            other => panic!("{name}: expect must be ALLOW or DENY, got {other}"),
        };
        let got = engine.decide(&p, a, &r).unwrap();
        if got.allowed != want {
            failures.push(format!("{name}: expected {}, got {got:?}", s["expect"]));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
