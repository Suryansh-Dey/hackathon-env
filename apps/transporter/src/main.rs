//! KabadOS transporter app (Dioxus desktop; Android via `dx bundle` + the
//! Kotlin bridge in PLAN §8.2). Drivers go on duty, accept job offers, and
//! work the pickup checklist. All state and money math come from the shared
//! `kabados-domain` / `kabados-pricing` / `kabados-client` crates, so the app
//! can never drift from the backend.

use dioxus::prelude::*;
use kabados_client::job::{Job, JobEventKind, JobStatus};
use kabados_client::models::{JobOfferView, JobView};
use kabados_client::{Client, ids::JobId};

const API_DEFAULT: &str = "http://localhost:8080";

fn main() {
    dioxus::launch(App);
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
}

#[component]
fn App() -> Element {
    let api_base = std::env::var("KABADOS_API").unwrap_or_else(|_| API_DEFAULT.to_string());
    let token = use_signal(|| Option::<String>::None);
    use_context_provider(|| Session { api_base, token });
    let signed_in = token.read().is_some();

    rsx! {
        style { {STYLES} }
        div { class: "app",
            header { class: "bar",
                span { class: "brand", "\u{267b} KabadOS Driver" }
                if signed_in {
                    button { class: "link", onclick: move |_| { let mut t = token; t.set(None); }, "Sign out" }
                }
            }
            main { class: "body",
                if signed_in {
                    Dashboard {}
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
        let api_base = session.api_base.clone();
        let mut token = session.token;
        spawn(async move {
            busy.set(true);
            error.set(None);
            // Dev sign-in: phone only, no OTP (POST /dev/token). Prod uses Cognito.
            let res = reqwest::Client::new()
                .post(format!("{api_base}/dev/token"))
                .json(&serde_json::json!({ "phone": phone() }))
                .send()
                .await;
            match res {
                Ok(r) if r.status().is_success() => match r.json::<serde_json::Value>().await {
                    Ok(v) => token.set(v["token"].as_str().map(str::to_owned)),
                    Err(e) => error.set(Some(e.to_string())),
                },
                Ok(_) => error.set(Some("Could not sign in with that number".into())),
                Err(e) => error.set(Some(e.to_string())),
            }
            busy.set(false);
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

#[component]
fn Dashboard() -> Element {
    let session = use_context::<Session>();
    let mut on_duty = use_signal(|| false);
    let open_job = use_signal(|| Option::<JobId>::None);

    // Poll job offers while on duty.
    let offers = {
        let session = session.clone();
        use_resource(move || {
            let session = session.clone();
            let duty = on_duty();
            async move {
                if !duty {
                    return Ok::<Vec<JobOfferView>, String>(vec![]);
                }
                session.client().job_offers().await.map_err(|e| e.to_string())
            }
        })
    };

    let toggle = {
        let session = session.clone();
        move |_| {
            let session = session.clone();
            let next = !on_duty();
            spawn(async move {
                if session.client().set_availability(next).await.is_ok() {
                    on_duty.set(next);
                }
            });
        }
    };

    if let Some(job_id) = open_job.read().clone() {
        return rsx! { JobChecklist { job_id, open_job } };
    }

    rsx! {
        div { class: "card",
            div { class: "between",
                div { h2 { "On duty" } p { class: "muted", "Only on-duty drivers receive job offers." } }
                button { class: if on_duty() { "btn" } else { "btn ghost" }, onclick: toggle,
                    if on_duty() { "On duty \u{2713}" } else { "Go on duty" }
                }
            }
        }
        if on_duty() {
            h3 { "Job offers" }
            match &*offers.read() {
                Some(Ok(list)) if list.is_empty() => rsx! { p { class: "empty", "No offers right now. Stay on duty." } },
                Some(Ok(list)) => rsx! {
                    div {
                        for o in list.clone() {
                            OfferCard { offer: o, open_job }
                        }
                    }
                },
                Some(Err(e)) => rsx! { p { class: "error", "{e}" } },
                None => rsx! { p { class: "muted", "Loading\u{2026}" } },
            }
        }
    }
}

#[component]
fn OfferCard(offer: JobOfferView, open_job: Signal<Option<JobId>>) -> Element {
    let session = use_context::<Session>();
    let job = offer.job.clone();
    let fare = format!("\u{20b9}{}", job.fare.0 / 100);

    let accept = {
        let session = session.clone();
        let id = job.id.clone();
        move |_| {
            let (session, id) = (session.clone(), id.clone());
            spawn(async move {
                let key = format!("accept-{id}");
                if session.client().accept_job(&id, &key).await.is_ok() {
                    open_job.set(Some(id));
                }
            });
        }
    };
    let decline = {
        let session = session.clone();
        let id = job.id.clone();
        move |_| {
            let (session, id) = (session.clone(), id.clone());
            spawn(async move {
                let key = format!("decline-{id}");
                let _ = session.client().decline_job(&id, &key).await;
            });
        }
    };

    rsx! {
        div { class: "card offer",
            div { class: "between",
                strong { "{offer.vendor_name}" }
                span { class: "pill", "{fare}" }
            }
            p { class: "muted", "Load ~{weight(&job)} \u{b7} {job.bulky_items} bulky \u{b7} floor {job.floors}" }
            div { class: "row",
                button { class: "btn", onclick: accept, "Accept" }
                button { class: "btn ghost", onclick: decline, "Decline" }
            }
        }
    }
}

fn weight(job: &Job) -> String {
    let kg = job.load_weight.0 as f64 / 1000.0;
    format!("{kg:.0} kg")
}

#[component]
fn JobChecklist(job_id: JobId, open_job: Signal<Option<JobId>>) -> Element {
    let session = use_context::<Session>();
    let mut refresh = use_signal(|| 0u32);
    let view = {
        let session = session.clone();
        let id = job_id.clone();
        use_resource(move || {
            let session = session.clone();
            let id = id.clone();
            let _ = refresh();
            async move { session.client().job(&id).await.map_err(|e| e.to_string()) }
        })
    };

    let do_event = {
        let session = session.clone();
        let id = job_id.clone();
        EventHandler::new(move |kind: JobEventKind| {
            let (session, id) = (session.clone(), id.clone());
            spawn(async move {
                let key = format!("evt-{id}-{kind:?}");
                let _ = session.client().post_job_event(&id, kind, None, None, &key).await;
                refresh.with_mut(|n| *n += 1);
            });
        })
    };

    rsx! {
        button { class: "link", onclick: move |_| open_job.set(None), "\u{2190} Back" }
        match &*view.read() {
            Some(Ok(jv)) => rsx! { JobSteps { jv: jv.clone(), do_event } },
            Some(Err(e)) => rsx! { p { class: "error", "{e}" } },
            None => rsx! { p { class: "muted", "Loading\u{2026}" } },
        }
    }
}

#[component]
fn JobSteps(jv: JobView, do_event: EventHandler<JobEventKind>) -> Element {
    let status = jv.job.status;
    // The next action offered depends on the job's state (PLAN §8.4 checklist).
    let next: Option<(&str, JobEventKind)> = match status {
        JobStatus::Accepted => Some(("Start navigating", JobEventKind::EnRoute)),
        JobStatus::EnRoute => Some(("I've arrived", JobEventKind::Arrived)),
        JobStatus::ArrivedPickup => Some(("Loaded the items", JobEventKind::Loaded)),
        JobStatus::Loaded => Some(("In transit to yard", JobEventKind::InTransit)),
        JobStatus::InTransit => Some(("Arrived at yard", JobEventKind::ArrivedYard)),
        JobStatus::ArrivedYard => Some(("Delivered", JobEventKind::Delivered)),
        _ => None,
    };

    rsx! {
        div { class: "card",
            h2 { "Pickup job" }
            p { class: "pill", "{status.as_str()}" }
            if let Some(addr) = jv.exact_address.as_ref() {
                div { class: "addr",
                    strong { "Pickup address" }
                    p { "{addr.text}" }
                    if let Some(l) = addr.landmark.as_ref() { p { class: "muted", "Landmark: {l}" } }
                }
            } else {
                p { class: "muted", "Address unlocks after you accept." }
            }
            div { class: "steps",
                for (label , done) in step_rows(status) {
                    div { class: if done { "step done" } else { "step" }, "{label}" }
                }
            }
            if let Some((label, kind)) = next {
                button { class: "btn big", onclick: move |_| do_event.call(kind), "{label}" }
            } else if status == JobStatus::Delivered || status == JobStatus::PaidOut {
                p { class: "ok", "Delivered. Thanks! Check your earnings." }
            }
        }
    }
}

/// Checklist rows with done/upcoming state derived from the job-status order.
fn step_rows(status: JobStatus) -> Vec<(&'static str, bool)> {
    let order = [
        (JobStatus::Accepted, "Accepted"),
        (JobStatus::EnRoute, "En route"),
        (JobStatus::ArrivedPickup, "Arrived at pickup"),
        (JobStatus::Loaded, "Loaded"),
        (JobStatus::InTransit, "In transit"),
        (JobStatus::ArrivedYard, "Arrived at yard"),
        (JobStatus::Delivered, "Delivered"),
    ];
    let rank = |s: JobStatus| order.iter().position(|(o, _)| *o == s).unwrap_or(0);
    let cur = rank(status);
    order.iter().map(|(s, label)| (*label, rank(*s) <= cur)).collect()
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
.empty { text-align: center; color: #9db0a2; padding: 28px; }
.input { width: 100%; padding: 12px; border-radius: 10px; border: 1px solid #263023; background: #10140f; color: #e9efe9; font-size: 16px; margin: 8px 0; }
.btn { background: #1f7a4d; color: #fff; border: none; border-radius: 10px; padding: 12px 18px; font-size: 16px; min-height: 48px; cursor: pointer; }
.btn.big { width: 100%; font-size: 18px; margin-top: 12px; }
.btn.ghost { background: transparent; border: 1px solid #3a473a; color: #e9efe9; }
.link { background: none; border: none; color: #3fae76; cursor: pointer; padding: 8px 0; font-size: 15px; }
.between { display: flex; justify-content: space-between; align-items: center; gap: 12px; }
.row { display: flex; gap: 10px; margin-top: 10px; }
.pill { background: #16271d; color: #3fae76; border-radius: 999px; padding: 4px 12px; font-size: 14px; }
.addr { background: #10140f; border-radius: 10px; padding: 12px; margin: 10px 0; }
.steps { margin: 12px 0; }
.step { padding: 8px 0 8px 26px; position: relative; color: #9db0a2; }
.step::before { content: "\25cb"; position: absolute; left: 4px; }
.step.done { color: #e9efe9; }
.step.done::before { content: "\25cf"; color: #3fae76; }
"#;
