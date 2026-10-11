//! KabadOS transporter app (Dioxus desktop). Drivers sign up, go on duty,
//! accept job offers, and work the pickup checklist: travel, weigh every
//! line, wait for the seller's confirmation and payment, verify the seller's
//! handover code, then carry the load to the vendor's yard. All state and
//! money math come from the shared `kabados-domain` / `kabados-client`
//! crates, so the app can never drift from the backend.

use dioxus::prelude::*;
use kabados_client::deal::{DealLine, DealStatus, LineStatus};
use kabados_client::ids::{DealId, JobId};
use kabados_client::job::{Job, JobEventKind, JobStatus};
use kabados_client::models::{ApplyTransporter, DealView, JobOfferView, JobView, MeView, PatchMe};
use kabados_client::offer::{PaymentMode, Rate};
use kabados_client::user::VehicleType;
use kabados_client::{Client, Grams, fresh_key};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The dev deployment; `KABADOS_API` (at build or run time) overrides it.
const API_DEFAULT: &str = "https://jl7jk1v3af.execute-api.ap-south-1.amazonaws.com";
const POLL_EVERY: Duration = Duration::from_secs(5);

fn main() {
    dioxus::launch(App);
}

fn api_base() -> String {
    std::env::var("KABADOS_API")
        .ok()
        .or_else(|| option_env!("KABADOS_API").map(str::to_owned))
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| API_DEFAULT.to_owned())
}

// ---- session (persisted so a restart keeps the driver signed in) ----

#[derive(Serialize, Deserialize)]
struct SavedSession {
    api_base: String,
    token: String,
}

fn session_file() -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))?;
    Some(dir.join("kabados-transporter").join("session.json"))
}

/// The saved token, if it was issued by the API this build talks to.
fn load_token(api_base: &str) -> Option<String> {
    let saved: SavedSession = serde_json::from_slice(&std::fs::read(session_file()?).ok()?).ok()?;
    (saved.api_base == api_base).then_some(saved.token)
}

fn write_private(path: &Path, body: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    opts.open(path)?.write_all(body)
}

#[derive(Clone)]
struct Session {
    api_base: String,
    token: Signal<Option<String>>,
}

impl Session {
    /// A client bound to the current token (cheap to rebuild per call).
    fn client(&self) -> Client {
        let c = Client::new(self.api_base.clone());
        match self.token.read().clone() {
            Some(t) => c.with_token(t),
            None => c,
        }
    }

    fn sign_in(&self, token: String) {
        if let Some(path) = session_file() {
            let saved = SavedSession { api_base: self.api_base.clone(), token: token.clone() };
            let res = serde_json::to_vec(&saved).map_err(std::io::Error::other).and_then(|b| write_private(&path, &b));
            if let Err(e) = res {
                tracing::warn!(error = %e, "could not save the session");
            }
        }
        let mut t = self.token;
        t.set(Some(token));
    }

    fn sign_out(&self) {
        if let Some(path) = session_file() {
            let _ = std::fs::remove_file(path);
        }
        let mut t = self.token;
        t.set(None);
    }
}

#[component]
fn App() -> Element {
    let api_base = use_hook(api_base);
    let token = use_signal({
        let api_base = api_base.clone();
        move || load_token(&api_base)
    });
    let session = use_context_provider(|| Session { api_base, token });
    let signed_in = token.read().is_some();

    rsx! {
        style { {STYLES} }
        div { class: "app",
            header { class: "bar",
                span { class: "brand", "\u{267b} KabadOS Driver" }
                if signed_in {
                    button { class: "link", onclick: move |_| session.sign_out(), "Sign out" }
                }
            }
            main { class: "body",
                if signed_in {
                    Home {}
                } else {
                    SignIn {}
                }
            }
        }
    }
}

#[component]
fn SignIn() -> Element {
    let session = use_context::<Session>();
    let mut phone = use_signal(|| "+91".to_string());
    let mut error = use_signal(|| Option::<String>::None);
    let mut busy = use_signal(|| false);

    let sign_in = move |_| {
        let session = session.clone();
        spawn(async move {
            busy.set(true);
            error.set(None);
            let res = Client::new(session.api_base.clone()).dev_token(phone().trim()).await;
            busy.set(false);
            match res {
                Ok(t) => session.sign_in(t.token),
                Err(e) => error.set(Some(e.message())),
            }
        });
    };

    rsx! {
        div { class: "card",
            h2 { "Driver sign-in" }
            p { class: "muted", "Enter your mobile number to go on duty." }
            input { class: "input", value: "{phone}", oninput: move |e| phone.set(e.value()), placeholder: "+91XXXXXXXXXX" }
            if let Some(e) = error() { p { class: "error", "{e}" } }
            button { class: "btn", disabled: busy(), onclick: sign_in, if busy() { "Signing in\u{2026}" } else { "Continue" } }
        }
    }
}

/// Loads the profile and routes to onboarding or the dashboard.
#[component]
fn Home() -> Element {
    let session = use_context::<Session>();
    let mut reload = use_signal(|| 0u32);
    let me = {
        let session = session.clone();
        use_resource(move || {
            let session = session.clone();
            let _ = reload();
            async move { session.client().me().await.map_err(|e| e.message()) }
        })
    };
    let state = me.read().clone();

    match state {
        None => rsx! { p { class: "muted", "Loading\u{2026}" } },
        Some(Err(e)) => rsx! {
            div { class: "card",
                p { class: "error", "{e}" }
                div { class: "row",
                    button { class: "btn", onclick: move |_| reload.with_mut(|n| *n += 1), "Retry" }
                    button { class: "btn ghost", onclick: move |_| session.sign_out(), "Sign in again" }
                }
            }
        },
        Some(Ok(me)) => match me.transporter {
            None => rsx! { Onboarding { me, on_done: move |_| reload.with_mut(|n| *n += 1) } },
            Some(t) => rsx! { Dashboard { on_duty: t.on_duty } },
        },
    }
}

const VEHICLES: [(&str, &str); 5] =
    [("CYCLE", "Cycle"), ("E_RICKSHAW", "E-rickshaw"), ("TEMPO", "Tempo"), ("MINI_TRUCK", "Mini truck"), ("TRUCK", "Truck")];

/// First run: name + 18+ (PATCH /v1/me), then the vehicle (TRANSPORTER:apply).
#[component]
fn Onboarding(me: MeView, on_done: EventHandler<()>) -> Element {
    let session = use_context::<Session>();
    let mut name = use_signal(|| me.name.clone().unwrap_or_default());
    let mut adult = use_signal(|| me.adult_confirmed);
    let mut vehicle = use_signal(|| "TEMPO".to_string());
    let mut payload = use_signal(|| "500".to_string());
    let mut registration = use_signal(String::new);
    let mut error = use_signal(|| Option::<String>::None);
    let mut busy = use_signal(|| false);

    let submit = move |_| {
        let client = session.client();
        spawn(async move {
            error.set(None);
            let name = name().trim().to_owned();
            let reg = registration().trim().to_owned();
            let Ok(vehicle_type) = serde_json::from_value::<VehicleType>(serde_json::Value::String(vehicle())) else {
                error.set(Some("Choose your vehicle type".into()));
                return;
            };
            let payload_kg = match payload().trim().parse::<u32>() {
                Ok(kg) if kg > 0 => kg,
                _ => {
                    error.set(Some("Enter how many kg your vehicle can carry".into()));
                    return;
                }
            };
            if name.is_empty() {
                error.set(Some("Enter your name".into()));
                return;
            }
            if !adult() {
                error.set(Some("You must be 18 or older to drive with KabadOS".into()));
                return;
            }
            if reg.is_empty() {
                error.set(Some("Enter your vehicle registration number".into()));
                return;
            }
            busy.set(true);
            let profile = PatchMe { name: Some(name), adult_confirmed: Some(true) };
            let apply = ApplyTransporter { vehicle_type, payload_kg, registration: reg, licence_number: None };
            let res = match client.patch_me(&profile, &fresh_key("me")).await {
                Ok(_) => client.apply_transporter(&apply, &fresh_key("apply")).await,
                Err(e) => Err(e),
            };
            busy.set(false);
            match res {
                Ok(_) => on_done.call(()),
                Err(e) => error.set(Some(e.message())),
            }
        });
    };

    rsx! {
        div { class: "card",
            h2 { "Set up your driver account" }
            p { class: "muted", "Signed in as \u{2022}\u{2022}\u{2022}\u{2022}{me.phone_last4}. Tell us about you and your vehicle." }
            label { class: "label", "Your name" }
            input { class: "input", value: "{name}", oninput: move |e| name.set(e.value()), placeholder: "Full name" }
            label { class: "check",
                input { r#type: "checkbox", checked: adult(), onchange: move |e| adult.set(e.checked()) }
                " I am 18 or older"
            }
            label { class: "label", "Vehicle type" }
            select { class: "input", value: "{vehicle}", onchange: move |e| vehicle.set(e.value()),
                for (value, text) in VEHICLES {
                    option { value, selected: vehicle() == value, "{text}" }
                }
            }
            label { class: "label", "Payload (kg)" }
            input { class: "input", value: "{payload}", inputmode: "numeric", oninput: move |e| payload.set(e.value()) }
            label { class: "label", "Registration number" }
            input { class: "input", value: "{registration}", oninput: move |e| registration.set(e.value()), placeholder: "MH12AB1234" }
            if let Some(e) = error() { p { class: "error", "{e}" } }
            button { class: "btn big", disabled: busy(), onclick: submit, if busy() { "Saving\u{2026}" } else { "Start driving" } }
        }
    }
}

#[component]
fn Dashboard(on_duty: bool) -> Element {
    let session = use_context::<Session>();
    let mut duty = use_signal(|| on_duty);
    let open_job = use_signal(|| Option::<JobId>::None);
    let mut error = use_signal(|| Option::<String>::None);
    let mut poll = use_signal(|| 0u64);
    let mut now = use_signal(chrono::Utc::now);

    // A 1 s clock for the offer countdowns; offers and jobs refetch every 5 s.
    use_future(move || async move {
        let mut ticks = 0u64;
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            now.set(chrono::Utc::now());
            ticks += 1;
            if ticks.is_multiple_of(POLL_EVERY.as_secs()) {
                poll.with_mut(|p| *p += 1);
            }
        }
    });

    let offers = {
        let session = session.clone();
        use_resource(move || {
            let session = session.clone();
            let (on, _) = (duty(), poll());
            async move {
                if !on {
                    return Ok::<Vec<JobOfferView>, String>(vec![]);
                }
                session.client().job_offers().await.map_err(|e| e.message())
            }
        })
    };

    // Accepted-but-unfinished jobs, so a job can be reopened after Back or a restart.
    let jobs = {
        let session = session.clone();
        use_resource(move || {
            let session = session.clone();
            let _ = poll();
            async move {
                let all = session.client().jobs(None).await.map_err(|e| e.message())?;
                Ok::<Vec<Job>, String>(all.into_iter().filter(|j| j.status.shares_location()).collect())
            }
        })
    };

    let toggle = move |_| {
        let session = session.clone();
        let next = !duty();
        spawn(async move {
            error.set(None);
            match session.client().set_availability(next).await {
                Ok(a) => duty.set(a.on_duty),
                Err(e) => error.set(Some(e.message())),
            }
        });
    };

    if let Some(job_id) = open_job() {
        return rsx! { JobChecklist { job_id, open_job } };
    }
    let now_ts = now().timestamp();

    rsx! {
        div { class: "card",
            div { class: "between",
                div { h2 { "On duty" } p { class: "muted", "Only on-duty drivers receive job offers." } }
                button { class: if duty() { "btn" } else { "btn ghost" }, onclick: toggle,
                    if duty() { "On duty \u{2713}" } else { "Go on duty" }
                }
            }
            if let Some(e) = error() { p { class: "error", "{e}" } }
        }
        h3 { "Active jobs" }
        match &*jobs.read() {
            Some(Ok(list)) if list.is_empty() => rsx! { p { class: "muted", "No active jobs." } },
            Some(Ok(list)) => rsx! {
                for j in list.clone() {
                    ActiveJobRow { key: "{j.id}", job: j, open_job }
                }
            },
            Some(Err(e)) => rsx! { p { class: "error", "{e}" } },
            None => rsx! { p { class: "muted", "Loading\u{2026}" } },
        }
        if duty() {
            h3 { "Job offers" }
            match &*offers.read() {
                Some(Ok(list)) if list.is_empty() => rsx! { p { class: "empty", "No offers right now. Stay on duty." } },
                Some(Ok(list)) => rsx! {
                    for o in list.clone() {
                        OfferCard { key: "{o.job.id}", offer: o, open_job, now_ts, on_change: move |_| poll.with_mut(|p| *p += 1) }
                    }
                },
                Some(Err(e)) => rsx! { p { class: "error", "{e}" } },
                None => rsx! { p { class: "muted", "Loading\u{2026}" } },
            }
        }
    }
}

#[component]
fn ActiveJobRow(job: Job, mut open_job: Signal<Option<JobId>>) -> Element {
    let id = job.id.clone();
    rsx! {
        div { class: "card",
            div { class: "between",
                div {
                    strong { "{job.status.as_str()}" }
                    p { class: "muted", "Fare {job.fare} \u{b7} load ~{weight(&job)}" }
                }
                button { class: "btn", onclick: move |_| open_job.set(Some(id.clone())), "Open" }
            }
        }
    }
}

#[component]
fn OfferCard(offer: JobOfferView, mut open_job: Signal<Option<JobId>>, now_ts: i64, on_change: EventHandler<()>) -> Element {
    let session = use_context::<Session>();
    let mut error = use_signal(|| Option::<String>::None);
    let mut busy = use_signal(|| false);
    let job = offer.job.clone();
    let left = (offer.expires_at.timestamp() - now_ts).max(0);

    let accept = {
        let session = session.clone();
        let id = job.id.clone();
        move |_| {
            let (session, id) = (session.clone(), id.clone());
            spawn(async move {
                busy.set(true);
                error.set(None);
                let res = session.client().accept_job(&id, &fresh_key("accept")).await;
                busy.set(false);
                match res {
                    Ok(_) => open_job.set(Some(id)),
                    Err(e) => {
                        error.set(Some(e.message()));
                        on_change.call(());
                    }
                }
            });
        }
    };
    let decline = {
        let id = job.id.clone();
        move |_| {
            let (session, id) = (session.clone(), id.clone());
            spawn(async move {
                busy.set(true);
                error.set(None);
                let res = session.client().decline_job(&id, &fresh_key("decline")).await;
                busy.set(false);
                if let Err(e) = res {
                    error.set(Some(e.message()));
                }
                on_change.call(());
            });
        }
    };

    rsx! {
        div { class: "card offer",
            div { class: "between",
                strong { "{offer.vendor_name}" }
                span { class: "pill", "{job.fare}" }
            }
            p { class: "muted", "Load ~{weight(&job)} \u{b7} {job.bulky_items} bulky \u{b7} floor {job.floors}" }
            p { class: if left > 0 { "muted" } else { "error" },
                if left > 0 { "Expires in {left / 60}:{left % 60:02}" } else { "Offer expired" }
            }
            if let Some(e) = error() { p { class: "error", "{e}" } }
            div { class: "row",
                button { class: "btn", disabled: busy() || left == 0, onclick: accept, "Accept" }
                button { class: "btn ghost", disabled: busy(), onclick: decline, "Decline" }
            }
        }
    }
}

fn weight(job: &Job) -> String {
    let kg = job.load_weight.0 as f64 / 1000.0;
    format!("{kg:.0} kg")
}

/// One checklist action; the job page runs it and refetches.
#[derive(Clone, PartialEq)]
enum Step {
    Event(JobEventKind),
    Weigh { deal: DealId, line_n: u32, grams: i64 },
    Handover { deal: DealId, otp: String },
}

#[component]
fn JobChecklist(job_id: JobId, mut open_job: Signal<Option<JobId>>) -> Element {
    let session = use_context::<Session>();
    let mut refresh = use_signal(|| 0u32);
    let mut error = use_signal(|| Option::<String>::None);
    let mut busy = use_signal(|| false);

    // Poll so the seller's confirmation and the payment show up on their own.
    use_future(move || async move {
        loop {
            tokio::time::sleep(POLL_EVERY).await;
            refresh.with_mut(|n| *n += 1);
        }
    });

    let data = {
        let session = session.clone();
        let id = job_id.clone();
        use_resource(move || {
            let session = session.clone();
            let id = id.clone();
            let _ = refresh();
            async move {
                let client = session.client();
                let jv = client.job(&id).await.map_err(|e| e.message())?;
                let dv = client.deal(&jv.job.deal_id).await.map_err(|e| e.message())?;
                Ok::<(JobView, DealView), String>((jv, dv))
            }
        })
    };

    let run = EventHandler::new(move |step: Step| {
        let (session, id) = (session.clone(), job_id.clone());
        spawn(async move {
            busy.set(true);
            error.set(None);
            let client = session.client();
            let res = match step {
                Step::Event(kind) => client.post_job_event(&id, kind, None, None, &fresh_key("evt")).await.map(|_| ()),
                Step::Weigh { deal, line_n, grams } => {
                    client.weigh(&deal, line_n, Grams(grams), None, &fresh_key("weigh")).await.map(|_| ())
                }
                Step::Handover { deal, otp } => client.handover(&deal, &otp, &fresh_key("handover")).await.map(|_| ()),
            };
            busy.set(false);
            if let Err(e) = res {
                error.set(Some(e.message()));
            }
            refresh.with_mut(|n| *n += 1);
        });
    });

    rsx! {
        button { class: "link", onclick: move |_| open_job.set(None), "\u{2190} Back" }
        if let Some(e) = error() { p { class: "error banner", "{e}" } }
        match &*data.read() {
            Some(Ok((jv, dv))) => rsx! { JobSteps { jv: jv.clone(), dv: dv.clone(), busy: busy(), run } },
            Some(Err(e)) => rsx! { p { class: "error", "{e}" } },
            None => rsx! { p { class: "muted", "Loading\u{2026}" } },
        }
    }
}

const STEPS: [&str; 10] = [
    "Accepted",
    "En route",
    "Arrived at pickup",
    "Items weighed",
    "Seller confirmed",
    "Payment received",
    "Handover code verified (loaded)",
    "In transit",
    "Arrived at yard",
    "Delivered (vendor confirmed)",
];

/// Index of the last completed checklist row.
fn stage(job: JobStatus, deal: DealStatus) -> usize {
    match job {
        JobStatus::Accepted => 0,
        JobStatus::EnRoute => 1,
        JobStatus::ArrivedPickup => match deal {
            DealStatus::Weighed | DealStatus::LineRegraded => 3,
            DealStatus::SellerConfirmed | DealStatus::AwaitingPayment => 4,
            DealStatus::Paid | DealStatus::PaidDirect => 5,
            _ => 2,
        },
        JobStatus::Loaded => 6,
        JobStatus::InTransit => 7,
        JobStatus::ArrivedYard => 8,
        JobStatus::Delivered | JobStatus::PaidOut => 9,
        _ => 0,
    }
}

#[component]
fn JobSteps(jv: JobView, dv: DealView, busy: bool, run: EventHandler<Step>) -> Element {
    let job = jv.job.clone();
    let deal = dv.deal.clone();
    let done = stage(job.status, deal.status);
    let event_button = |label: &'static str, kind: JobEventKind| {
        rsx! { button { class: "btn big", disabled: busy, onclick: move |_| run.call(Step::Event(kind)), "{label}" } }
    };
    let amount = deal.final_amount.map(|a| a.to_string()).unwrap_or_default();

    let next = match job.status {
        JobStatus::Accepted => event_button("Start navigating", JobEventKind::EnRoute),
        JobStatus::EnRoute => event_button("I've arrived", JobEventKind::Arrived),
        JobStatus::ArrivedPickup => match deal.status {
            DealStatus::AtPickup | DealStatus::Weighed | DealStatus::LineRegraded => rsx! {
                h3 { "Weigh each item" }
                for line in dv.lines.clone() {
                    LineRow { key: "{line.n}", deal_id: deal.id.clone(), line, busy, run }
                }
                if let Some(total) = deal.final_amount { p { class: "total", "Total for the seller: {total}" } }
                p { class: "muted",
                    match deal.status {
                        DealStatus::Weighed => "All items weighed. Waiting for the seller to confirm the weights in their app\u{2026}",
                        DealStatus::LineRegraded => "Waiting for the seller to accept or remove a re-graded item\u{2026}",
                        _ => "Enter the scale reading for every item.",
                    }
                }
            },
            DealStatus::SellerConfirmed | DealStatus::AwaitingPayment => rsx! {
                p { class: "muted",
                    match deal.payment_mode {
                        PaymentMode::Platform => format!("Seller confirmed {amount}. Waiting for the vendor's payment\u{2026}"),
                        PaymentMode::Direct => format!("Seller confirmed {amount}. Waiting for the vendor to pay the seller and the seller to confirm\u{2026}"),
                    }
                }
            },
            DealStatus::Paid | DealStatus::PaidDirect => rsx! { HandoverForm { deal_id: deal.id.clone(), busy, run } },
            other => rsx! { p { class: "muted", "Deal is {other.as_str()}." } },
        },
        JobStatus::Loaded => event_button("In transit to yard", JobEventKind::InTransit),
        JobStatus::InTransit => event_button("Arrived at yard", JobEventKind::ArrivedYard),
        JobStatus::ArrivedYard => rsx! { p { class: "muted", "Waiting for the vendor to confirm delivery at the yard\u{2026}" } },
        JobStatus::Delivered | JobStatus::PaidOut => rsx! { p { class: "ok", "Delivered. Thanks! Check your earnings." } },
        other => rsx! { p { class: "muted", "This job is {other.as_str()}." } },
    };

    rsx! {
        div { class: "card",
            div { class: "between",
                h2 { "Pickup job" }
                span { class: "pill", "{job.status.as_str()}" }
            }
            p { class: "muted", "{dv.listing.title} \u{b7} fare {job.fare} \u{b7} deal {deal.status.as_str()}" }
            if let Some(name) = dv.counterpart_name.as_ref() { p { class: "muted", "Seller: {name}" } }
            if let Some(addr) = jv.exact_address.as_ref() {
                div { class: "addr",
                    strong { "Pickup address" }
                    p { "{addr.text}" }
                    if let Some(l) = addr.landmark.as_ref() { p { class: "muted", "Landmark: {l}" } }
                }
            } else if done < 6 {
                p { class: "muted", "Address unlocks after you accept." }
            }
            div { class: "steps",
                for (i , label) in STEPS.iter().enumerate() {
                    div { class: if i <= done { "step done" } else { "step" }, "{label}" }
                }
            }
            {next}
        }
    }
}

fn rate_text(rate: &Rate) -> String {
    match rate {
        Rate::Kg { paise_per_kg } => format!("{paise_per_kg}/kg"),
        Rate::Piece { paise_per_piece } => format!("{paise_per_piece}/piece"),
    }
}

/// "12.5" kg → grams.
fn parse_kg(s: &str) -> Option<i64> {
    let kg: f64 = s.trim().parse().ok()?;
    let grams = (kg * 1000.0).round();
    (kg.is_finite() && (1.0..=50_000_000.0).contains(&grams)).then_some(grams as i64)
}

#[component]
fn LineRow(deal_id: DealId, line: DealLine, busy: bool, run: EventHandler<Step>) -> Element {
    let mut kg = use_signal(|| line.weighed_g.map(|g| format!("{:.1}", g.0 as f64 / 1000.0)).unwrap_or_default());
    let mut error = use_signal(|| Option::<String>::None);
    let n = line.n;
    let save = move |_| match parse_kg(&kg()) {
        Some(grams) => {
            error.set(None);
            run.call(Step::Weigh { deal: deal_id.clone(), line_n: n, grams });
        }
        None => error.set(Some("Enter the scale reading in kg, e.g. 12.5".into())),
    };
    let weighed = line.weighed_g.map(|g| g.to_string()).unwrap_or_else(|| "not yet".into());
    let amount = line.amount.map(|a| a.to_string()).unwrap_or_else(|| "after every item is weighed".into());
    let grade = line.grade.as_deref().map(|g| format!(" ({g})")).unwrap_or_default();

    rsx! {
        div { class: "line",
            div { class: "between",
                strong { "#{n} {line.category_id}{grade}" }
                span { class: "muted", "\u{d7}{line.quantity} \u{b7} {rate_text(&line.rate)}" }
            }
            if line.status == LineStatus::Removed {
                p { class: "muted", "Removed by the seller." }
            } else {
                div { class: "row",
                    input { class: "input", value: "{kg}", inputmode: "decimal", placeholder: "kg", oninput: move |e| kg.set(e.value()) }
                    button { class: "btn", disabled: busy, onclick: save, if line.weighed_g.is_some() { "Update" } else { "Save" } }
                }
                p { class: "muted", "Weighed: {weighed} \u{b7} Amount: {amount}" }
                if line.status == LineStatus::RegradePending { p { class: "muted", "Re-graded \u{2014} waiting for the seller." } }
            }
            if let Some(e) = error() { p { class: "error", "{e}" } }
        }
    }
}

#[component]
fn HandoverForm(deal_id: DealId, busy: bool, run: EventHandler<Step>) -> Element {
    let mut otp = use_signal(String::new);
    let ready = otp.read().len() == 4;
    rsx! {
        h3 { "Handover" }
        p { class: "muted", "Payment is done. Ask the seller for the 4-digit handover code shown in their app, then load the items." }
        input { class: "input code", value: "{otp}", inputmode: "numeric", maxlength: "4", placeholder: "\u{2022}\u{2022}\u{2022}\u{2022}",
            oninput: move |e| otp.set(e.value().chars().filter(|c| c.is_ascii_digit()).take(4).collect())
        }
        button { class: "btn big", disabled: busy || !ready, onclick: move |_| run.call(Step::Handover { deal: deal_id.clone(), otp: otp() }),
            "Verify code & load"
        }
    }
}

const STYLES: &str = r#"
* { box-sizing: border-box; font-family: system-ui, sans-serif; }
body { margin: 0; background: #0f130e; color: #e9efe9; }
.app { max-width: 520px; margin: 0 auto; }
.bar { display: flex; justify-content: space-between; align-items: center; padding: 14px 16px; border-bottom: 1px solid #263023; }
.brand { font-weight: 700; color: #3fae76; }
.body { padding: 16px; }
.card { background: #171c15; border: 1px solid #263023; border-radius: 14px; padding: 16px; margin-bottom: 14px; }
.card.offer { border-color: #2f6b49; }
h2 { margin: 0 0 6px; } h3 { margin: 18px 0 8px; }
.muted { color: #9db0a2; } .error { color: #ff8a8a; } .ok { color: #3fae76; }
.error.banner { background: #2a1515; border-radius: 10px; padding: 10px 12px; }
.empty { text-align: center; color: #9db0a2; padding: 28px; }
.label { display: block; margin-top: 10px; color: #9db0a2; font-size: 14px; }
.check { display: flex; align-items: center; gap: 8px; margin: 12px 0; }
.check input { width: 20px; height: 20px; }
.input { width: 100%; padding: 12px; border-radius: 10px; border: 1px solid #263023; background: #10140f; color: #e9efe9; font-size: 16px; margin: 8px 0; }
.input.code { font-size: 28px; letter-spacing: 12px; text-align: center; }
.row .input { margin: 0; }
.btn { background: #1f7a4d; color: #fff; border: none; border-radius: 10px; padding: 12px 18px; font-size: 16px; min-height: 48px; cursor: pointer; }
.btn:disabled { opacity: 0.5; cursor: default; }
.btn.big { width: 100%; font-size: 18px; margin-top: 12px; }
.btn.ghost { background: transparent; border: 1px solid #3a473a; color: #e9efe9; }
.link { background: none; border: none; color: #3fae76; cursor: pointer; padding: 8px 0; font-size: 15px; }
.between { display: flex; justify-content: space-between; align-items: center; gap: 12px; }
.row { display: flex; gap: 10px; margin-top: 10px; align-items: center; }
.pill { background: #16271d; color: #3fae76; border-radius: 999px; padding: 4px 12px; font-size: 14px; white-space: nowrap; }
.addr { background: #10140f; border-radius: 10px; padding: 12px; margin: 10px 0; }
.line { background: #10140f; border-radius: 10px; padding: 12px; margin: 10px 0; }
.total { font-size: 18px; font-weight: 600; }
.steps { margin: 12px 0; }
.step { padding: 8px 0 8px 26px; position: relative; color: #9db0a2; }
.step::before { content: "\25cb"; position: absolute; left: 4px; }
.step.done { color: #e9efe9; }
.step.done::before { content: "\25cf"; color: #3fae76; }
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scale_readings_in_kg() {
        assert_eq!(parse_kg("12.5"), Some(12_500));
        assert_eq!(parse_kg(" 0.001 "), Some(1));
        assert_eq!(parse_kg("0"), None);
        assert_eq!(parse_kg("-3"), None);
        assert_eq!(parse_kg("abc"), None);
        assert_eq!(parse_kg("60000"), None);
    }

    #[test]
    fn checklist_stage_follows_job_then_deal() {
        assert_eq!(stage(JobStatus::Accepted, DealStatus::JobAssigned), 0);
        assert_eq!(stage(JobStatus::ArrivedPickup, DealStatus::AtPickup), 2);
        assert_eq!(stage(JobStatus::ArrivedPickup, DealStatus::Weighed), 3);
        assert_eq!(stage(JobStatus::ArrivedPickup, DealStatus::AwaitingPayment), 4);
        assert_eq!(stage(JobStatus::ArrivedPickup, DealStatus::PaidDirect), 5);
        assert_eq!(stage(JobStatus::Loaded, DealStatus::HandedOver), 6);
        assert_eq!(stage(JobStatus::Delivered, DealStatus::Settled), STEPS.len() - 1);
    }
}
