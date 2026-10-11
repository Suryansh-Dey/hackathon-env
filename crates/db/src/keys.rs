//! Every key format in one place (PLAN.md §16.4, §16.6, §16.7).

use kabados_domain::ids::*;

pub const META: &str = "META";

/// Zero-padded so lexical order equals numeric order.
pub fn pad(n: u32) -> String {
    format!("{n:010}")
}

pub fn user(id: &UserId) -> String {
    format!("USER#{id}")
}
pub const PROFILE: &str = "PROFILE";
pub fn cognito(sub: &str) -> String {
    format!("COGNITO#{sub}")
}
pub fn phone(hash: &str) -> String {
    format!("PHONE#{hash}")
}
pub const PHONE: &str = "PHONE";
pub fn role(r: &str) -> String {
    format!("ROLE#{r}")
}
pub fn addr(id: &AddressId) -> String {
    format!("ADDR#{id}")
}
pub fn payout(id: &PayoutMethodId) -> String {
    format!("PAYOUT#{id}")
}
pub fn consent(purpose: &str, ver: &str) -> String {
    format!("CONSENT#{purpose}#{ver}")
}
pub fn device(id: &str) -> String {
    format!("DEVICE#{id}")
}
pub const VENDOR: &str = "VENDOR";
pub const TRANSPORTER: &str = "TRANSPORTER";
pub fn kyc_queue(status: &str) -> String {
    format!("KYC#{status}")
}
pub fn doc(id: &DocumentId) -> String {
    format!("DOC#{id}")
}
pub fn vehicle(id: &VehicleId) -> String {
    format!("VEHICLE#{id}")
}
pub fn my_transporter(t: &UserId) -> String {
    format!("MYTRANSPORTER#{t}")
}
pub fn my_vendor(v: &UserId) -> String {
    format!("MYVENDOR#{v}")
}
pub fn ruleset(version: u32) -> String {
    format!("RULESET#{}", pad(version))
}
pub const RULESET_PREFIX: &str = "RULESET#";
pub const RULESET_DRAFT: &str = "RULESETDRAFT";
pub const COVERAGE_MANIFEST: &str = "COVMANIFEST";
pub fn coverage(cell_r8: &str, category: &str) -> String {
    format!("COV#{cell_r8}#{category}")
}
pub fn vendor_sk(v: &UserId) -> String {
    format!("VENDOR#{v}")
}

pub fn listing(id: &ListingId) -> String {
    format!("LISTING#{id}")
}
pub fn listing_version(ver: u32) -> String {
    format!("V#{}", pad(ver))
}
pub fn version_prefix(ver: u32) -> String {
    format!("V#{}#", pad(ver))
}
pub fn item(ver: u32, id: &ItemId) -> String {
    format!("V#{}#ITEM#{id}", pad(ver))
}
pub fn claim(ver: u32, id: &ClaimId) -> String {
    format!("V#{}#CLAIM#{id}", pad(ver))
}
pub fn proof(ver: u32, id: &ProofRequestId) -> String {
    format!("V#{}#PROOF#{id}", pad(ver))
}
pub fn image(ver: u32, id: &ImageId) -> String {
    format!("V#{}#IMG#{id}", pad(ver))
}
pub fn match_result(ver: u32) -> String {
    format!("V#{}#MATCH", pad(ver))
}
pub fn feed(cell_r7: &str) -> String {
    format!("FEED#{cell_r7}")
}
pub fn listing_by_status(status: &str, ts: &str) -> String {
    format!("LISTING#{status}#{ts}")
}
pub fn offer(id: &OfferId) -> String {
    format!("OFFER#{id}")
}
pub const OFFER_PREFIX: &str = "OFFER#";
pub fn offer_by_status(status: &str, ts: &str) -> String {
    format!("OFFER#{status}#{ts}")
}
pub fn phash(band: u8, value: u16) -> String {
    format!("PHASH#{band}#{value:04x}")
}
pub fn img_sk(id: &ImageId) -> String {
    format!("IMG#{id}")
}

pub fn deal(id: &DealId) -> String {
    format!("DEAL#{id}")
}
pub fn deal_by_status(status: &str, ts: &str) -> String {
    format!("DEAL#{status}#{ts}")
}
pub fn deal_index(id: &DealId, role: &str) -> String {
    format!("DEALIDX#{role}#{id}")
}
pub fn line(n: u32) -> String {
    format!("LINE#{n:04}")
}
pub fn weighing(n: u32, ts: &str) -> String {
    format!("LINE#{n:04}#WEIGH#{ts}")
}
pub fn job(id: &JobId) -> String {
    format!("JOB#{id}")
}
pub fn job_by_status(status: &str, ts: &str) -> String {
    format!("JOB#{status}#{ts}")
}
pub fn job_offer_index(id: &JobId) -> String {
    format!("JOBOFFER#{id}")
}
pub fn job_event(id: &JobId, ts: &str) -> String {
    format!("JOB#{id}#EVT#{ts}")
}
pub fn call(id: &CallId) -> String {
    format!("CALL#{id}")
}
pub fn dispute(id: &DisputeId) -> String {
    format!("DISPUTE#{id}")
}
pub fn dispute_queue(status: &str) -> String {
    format!("DISPUTE#{status}")
}
pub fn rating(from: &UserId) -> String {
    format!("RATING#{from}")
}
pub fn receipt(id: &ReceiptId) -> String {
    format!("RECEIPT#{id}")
}

pub fn drive(id: &DriveId) -> String {
    format!("DRIVE#{id}")
}
pub fn drive_member(l: &ListingId) -> String {
    format!("MEMBER#{l}")
}
pub fn drive_by_date(date: &str) -> String {
    format!("DRIVE#{date}")
}
pub fn drive_code(code: &str) -> String {
    format!("DRIVECODE#{code}")
}

pub const TAXONOMY: &str = "TAXONOMY";
pub fn category(id: &str) -> String {
    format!("CAT#{id}")
}
pub fn recycler(state: &str) -> String {
    format!("RECYCLER#{state}")
}
pub fn recycler_auth(no: &str) -> String {
    format!("AUTH#{no}")
}
pub fn rate(city: &str, category: &str, grade: &str) -> String {
    format!("RATE#{city}#{category}#{grade}")
}
pub fn week(iso_week: &str) -> String {
    format!("WEEK#{iso_week}")
}
pub fn rate_city(city: &str) -> String {
    format!("RATECITY#{city}")
}
pub fn impact_city(city: &str) -> String {
    format!("IMPACT#{city}")
}
pub fn outbox(ts: &str, id: &EventId) -> String {
    format!("OUTBOX#{ts}#{id}")
}
pub fn notify(ts: &str, id: &NotificationId) -> String {
    format!("NOTIFY#{ts}#{id}")
}

pub fn thread(id: &ThreadId) -> String {
    format!("THREAD#{id}")
}
pub fn thread_for(listing: &ListingId, vendor: &UserId) -> String {
    format!("THREADFOR#{listing}#{vendor}")
}
pub fn message_sk(ts: &str, id: &MessageId) -> String {
    format!("{ts}#{id}")
}

// ledger table
pub fn txn(key: &str) -> String {
    format!("TXN#{key}")
}
pub fn entry(n: u32) -> String {
    format!("ENTRY#{n:03}")
}
pub fn account(id: &str) -> String {
    format!("ACCT#{id}")
}
pub const BALANCE: &str = "BALANCE";
pub fn account_history(ts: &str, key: &str) -> String {
    format!("TXN#{ts}#{key}")
}

// ephemeral table
pub fn idem(scope: &str, key: &str) -> String {
    format!("IDEM#{scope}#{key}")
}
pub const IDEM: &str = "IDEM";
pub fn otp_channel(phone_hash: &str) -> String {
    format!("OTPCH#{phone_hash}")
}
pub const OTPCH: &str = "OTPCH";
pub fn match_cache(key: &str) -> String {
    format!("MATCH#{key}")
}
pub const MATCH: &str = "MATCH";
pub fn rate_limit(scope: &str, subject: &str, window: &str) -> String {
    format!("RL#{scope}#{subject}#{window}")
}
pub const RL: &str = "RL";
pub fn inbox(env: &str) -> String {
    format!("INBOX#{env}")
}

// audit table
pub fn subject(id: &str) -> String {
    format!("SUBJECT#{id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_sort_numerically() {
        assert!(ruleset(9) < ruleset(10));
        assert!(listing_version(2) < listing_version(11));
        assert!(item(3, &ItemId::new()).starts_with(&version_prefix(3)));
        assert!(!item(31, &ItemId::new()).starts_with(&version_prefix(3)));
    }
}
