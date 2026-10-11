use async_trait::async_trait;
use serde_json::{Map, Value};
use thiserror::Error;

/// A stored item: top-level attributes as JSON.
pub type Item = Map<String, Value>;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Table {
    Core,
    Ledger,
    Chat,
    Ephemeral,
    Audit,
}

impl Table {
    pub const ALL: [Table; 5] = [Table::Core, Table::Ledger, Table::Chat, Table::Ephemeral, Table::Audit];

    pub fn suffix(self) -> &'static str {
        match self {
            Table::Core => "core",
            Table::Ledger => "ledger",
            Table::Chat => "chat",
            Table::Ephemeral => "ephemeral",
            Table::Audit => "audit",
        }
    }

    /// Only `core` has GSIs (PLAN.md §16.3).
    pub fn has_gsis(self) -> bool {
        self == Table::Core
    }
}

/// Physical table names, from env `TABLE_CORE`, … or `<prefix>-<suffix>`.
#[derive(Clone, Debug)]
pub struct TableNames {
    names: [String; 5],
}

impl TableNames {
    pub fn with_prefix(prefix: &str) -> Self {
        Self { names: Table::ALL.map(|t| format!("{prefix}-{}", t.suffix())) }
    }

    pub fn from_env() -> Self {
        let prefix = std::env::var("TABLE_PREFIX").unwrap_or_else(|_| "kabados-local".into());
        let mut n = Self::with_prefix(&prefix);
        for (i, t) in Table::ALL.iter().enumerate() {
            if let Ok(v) = std::env::var(format!("TABLE_{}", t.suffix().to_uppercase())) {
                n.names[i] = v;
            }
        }
        n
    }

    pub fn name(&self, t: Table) -> &str {
        &self.names[Table::ALL.iter().position(|x| *x == t).unwrap_or(0)]
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Index {
    Gsi1,
    Gsi2,
    Gsi3,
}

impl Index {
    pub fn name(self) -> &'static str {
        match self {
            Index::Gsi1 => "GSI1",
            Index::Gsi2 => "GSI2",
            Index::Gsi3 => "GSI3",
        }
    }

    pub fn pk_attr(self) -> &'static str {
        match self {
            Index::Gsi1 => "GSI1PK",
            Index::Gsi2 => "GSI2PK",
            Index::Gsi3 => "GSI3PK",
        }
    }

    pub fn sk_attr(self) -> &'static str {
        match self {
            Index::Gsi1 => "GSI1SK",
            Index::Gsi2 => "GSI2SK",
            Index::Gsi3 => "GSI3SK",
        }
    }
}

/// Condition evaluated against the existing item (absent item = no attributes).
#[derive(Clone, Debug, PartialEq)]
pub enum Cond {
    /// `attribute_not_exists(PK)`: the item must not exist.
    NotExists,
    /// `attribute_exists(PK)`: the item must exist.
    Exists,
    /// Top-level attribute equals value.
    Eq(String, Value),
    /// Top-level attribute differs from value (or is absent).
    Ne(String, Value),
    /// Top-level attribute is one of the values.
    In(String, Vec<Value>),
    And(Vec<Cond>),
}

impl Cond {
    pub fn version(v: u64) -> Cond {
        Cond::Eq("version".into(), Value::from(v))
    }

    pub fn status(s: &str) -> Cond {
        Cond::Eq("status".into(), Value::from(s))
    }

    pub fn and(self, other: Cond) -> Cond {
        match self {
            Cond::And(mut v) => {
                v.push(other);
                Cond::And(v)
            }
            c => Cond::And(vec![c, other]),
        }
    }

    /// Evaluates the condition (used by the memory backend and tests).
    pub fn eval(&self, existing: Option<&Item>) -> bool {
        match self {
            Cond::NotExists => existing.is_none(),
            Cond::Exists => existing.is_some(),
            Cond::Eq(a, v) => existing.and_then(|i| i.get(a)) == Some(v),
            Cond::Ne(a, v) => existing.and_then(|i| i.get(a)) != Some(v),
            Cond::In(a, vs) => existing.and_then(|i| i.get(a)).is_some_and(|x| vs.contains(x)),
            Cond::And(cs) => cs.iter().all(|c| c.eval(existing)),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SkCond {
    Eq(String),
    BeginsWith(String),
    Between(String, String),
    Gte(String),
    Lte(String),
}

impl SkCond {
    pub fn matches(&self, sk: &str) -> bool {
        match self {
            SkCond::Eq(s) => sk == s,
            SkCond::BeginsWith(p) => sk.starts_with(p.as_str()),
            SkCond::Between(a, b) => sk >= a.as_str() && sk <= b.as_str(),
            SkCond::Gte(a) => sk >= a.as_str(),
            SkCond::Lte(b) => sk <= b.as_str(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Query {
    pub table: Table,
    pub index: Option<Index>,
    pub pk: String,
    pub sk: Option<SkCond>,
    pub limit: Option<u32>,
    pub forward: bool,
    pub cursor: Option<String>,
}

impl Query {
    pub fn new(table: Table, pk: impl Into<String>) -> Self {
        Self { table, index: None, pk: pk.into(), sk: None, limit: None, forward: true, cursor: None }
    }

    pub fn index(mut self, i: Index) -> Self {
        self.index = Some(i);
        self
    }

    pub fn sk(mut self, c: SkCond) -> Self {
        self.sk = Some(c);
        self
    }

    pub fn begins_with(self, p: impl Into<String>) -> Self {
        self.sk(SkCond::BeginsWith(p.into()))
    }

    pub fn limit(mut self, n: u32) -> Self {
        self.limit = Some(n);
        self
    }

    pub fn reverse(mut self) -> Self {
        self.forward = false;
        self
    }

    pub fn cursor(mut self, c: Option<String>) -> Self {
        self.cursor = c;
        self
    }
}

#[derive(Clone, Debug, Default)]
pub struct Page {
    pub items: Vec<Item>,
    pub cursor: Option<String>,
}

#[derive(Clone, Debug)]
pub enum TxOp {
    Put { table: Table, item: Item, cond: Option<Cond> },
    Delete { table: Table, pk: String, sk: String, cond: Option<Cond> },
    Check { table: Table, pk: String, sk: String, cond: Cond },
}

#[derive(Debug, Error)]
pub enum DbError {
    #[error("condition check failed")]
    ConditionFailed,
    #[error("transaction cancelled: {0:?}")]
    TransactionCancelled(Vec<Option<String>>),
    #[error("serialization: {0}")]
    Serde(String),
    #[error("invalid cursor")]
    BadCursor,
    #[error("too many operations in one transaction ({0} > 100)")]
    TooManyOps(usize),
    #[error("backend: {0}")]
    Backend(String),
}

impl DbError {
    /// True when a conditional write lost a race (or a uniqueness check hit).
    pub fn is_conflict(&self) -> bool {
        matches!(self, DbError::ConditionFailed | DbError::TransactionCancelled(_))
    }
}

impl From<serde_json::Error> for DbError {
    fn from(e: serde_json::Error) -> Self {
        DbError::Serde(e.to_string())
    }
}

pub type DbResult<T> = Result<T, DbError>;

/// Streams-like change record (DynamoDB `NEW_AND_OLD_IMAGES`).
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub table: Table,
    pub old: Option<Item>,
    pub new: Option<Item>,
}

/// The storage port.
#[async_trait]
pub trait Store: Send + Sync {
    async fn get(&self, table: Table, pk: &str, sk: &str) -> DbResult<Option<Item>>;
    async fn put(&self, table: Table, item: Item, cond: Option<Cond>) -> DbResult<()>;
    async fn delete(&self, table: Table, pk: &str, sk: &str, cond: Option<Cond>) -> DbResult<()>;
    async fn query(&self, q: Query) -> DbResult<Page>;
    /// All-or-nothing write of up to 100 operations.
    async fn transact(&self, ops: Vec<TxOp>) -> DbResult<()>;

    /// Batch get (≤ 100 keys); missing items are skipped.
    async fn batch_get(&self, table: Table, keys: &[(String, String)]) -> DbResult<Vec<Item>> {
        let mut out = Vec::with_capacity(keys.len());
        for (pk, sk) in keys {
            if let Some(i) = self.get(table, pk, sk).await? {
                out.push(i);
            }
        }
        Ok(out)
    }

    /// Unconditional batch writes (puts and deletes), chunked by the backend.
    async fn batch_write(&self, table: Table, puts: Vec<Item>, deletes: Vec<(String, String)>) -> DbResult<()> {
        for i in puts {
            self.put(table, i, None).await?;
        }
        for (pk, sk) in deletes {
            self.delete(table, &pk, &sk, None).await?;
        }
        Ok(())
    }

    /// Reads every page of a query.
    async fn query_all(&self, mut q: Query) -> DbResult<Vec<Item>> {
        let mut out = Vec::new();
        loop {
            let page = self.query(q.clone()).await?;
            out.extend(page.items);
            match page.cursor {
                Some(c) => q.cursor = Some(c),
                None => return Ok(out),
            }
        }
    }
}

pub fn str_attr<'a>(item: &'a Item, name: &str) -> Option<&'a str> {
    item.get(name).and_then(Value::as_str)
}
