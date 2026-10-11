//! Road distance/ETA (Amazon Location Routes) and live positions (Trackers), PLAN.md §7.5, §8.4.

use async_trait::async_trait;
use kabados_domain::geo::LatLng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Leg {
    pub distance_m: i64,
    pub duration_s: i64,
}

#[async_trait]
pub trait RoutesPort: Send + Sync {
    /// Origins × destinations matrix (row per origin).
    async fn matrix(&self, origins: &[LatLng], destinations: &[LatLng]) -> anyhow::Result<Vec<Vec<Option<Leg>>>>;

    async fn leg(&self, from: LatLng, to: LatLng) -> anyhow::Result<Option<Leg>> {
        Ok(self.matrix(&[from], &[to]).await?.first().and_then(|r| r.first().copied().flatten()))
    }
}

pub struct AwsRoutes {
    client: aws_sdk_georoutes::Client,
}

impl AwsRoutes {
    pub fn new(client: aws_sdk_georoutes::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl RoutesPort for AwsRoutes {
    async fn matrix(&self, origins: &[LatLng], destinations: &[LatLng]) -> anyhow::Result<Vec<Vec<Option<Leg>>>> {
        use aws_sdk_georoutes::types::{RouteMatrixBoundary, RouteMatrixDestination, RouteMatrixOrigin, RouteMatrixTravelMode};
        let mut req = self
            .client
            .calculate_route_matrix()
            .travel_mode(RouteMatrixTravelMode::Car)
            .routing_boundary(RouteMatrixBoundary::builder().unbounded(true).build());
        for o in origins {
            req = req.origins(RouteMatrixOrigin::builder().position(o.lng).position(o.lat).build()?);
        }
        for d in destinations {
            req = req.destinations(RouteMatrixDestination::builder().position(d.lng).position(d.lat).build()?);
        }
        let out = req.send().await?;
        Ok(out
            .route_matrix()
            .iter()
            .map(|row| {
                row.iter().map(|e| (e.error().is_none()).then(|| Leg { distance_m: e.distance(), duration_s: e.duration() })).collect()
            })
            .collect())
    }
}

/// Haversine × 1.3 at 20 km/h: offline approximation for local development.
pub struct FakeRoutes;

#[async_trait]
impl RoutesPort for FakeRoutes {
    async fn matrix(&self, origins: &[LatLng], destinations: &[LatLng]) -> anyhow::Result<Vec<Vec<Option<Leg>>>> {
        Ok(origins
            .iter()
            .map(|o| {
                destinations
                    .iter()
                    .map(|d| {
                        let m = (o.distance_m(d) * 1.3).round() as i64;
                        Some(Leg { distance_m: m, duration_s: m * 18 / 100 })
                    })
                    .collect()
            })
            .collect())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Position {
    pub lat: f64,
    pub lng: f64,
    pub at: chrono::DateTime<chrono::Utc>,
    pub accuracy_m: Option<f64>,
}

#[async_trait]
pub trait TrackerPort: Send + Sync {
    async fn update(&self, device_id: &str, positions: &[Position]) -> anyhow::Result<()>;
    async fn latest(&self, device_id: &str) -> anyhow::Result<Option<Position>>;
}

pub struct AwsTracker {
    client: aws_sdk_location::Client,
    tracker: String,
}

impl AwsTracker {
    pub fn new(client: aws_sdk_location::Client, tracker: String) -> Self {
        Self { client, tracker }
    }
}

#[async_trait]
impl TrackerPort for AwsTracker {
    async fn update(&self, device_id: &str, positions: &[Position]) -> anyhow::Result<()> {
        use aws_sdk_location::types::{DevicePositionUpdate, PositionalAccuracy};
        for chunk in positions.chunks(10) {
            let mut req = self.client.batch_update_device_position().tracker_name(&self.tracker);
            for p in chunk {
                let mut u = DevicePositionUpdate::builder()
                    .device_id(device_id)
                    .position(p.lng)
                    .position(p.lat)
                    .sample_time(aws_sdk_location::primitives::DateTime::from_secs(p.at.timestamp()));
                if let Some(a) = p.accuracy_m {
                    u = u.accuracy(PositionalAccuracy::builder().horizontal(a).build()?);
                }
                req = req.updates(u.build()?);
            }
            req.send().await?;
        }
        Ok(())
    }

    async fn latest(&self, device_id: &str) -> anyhow::Result<Option<Position>> {
        let out = self.client.get_device_position().tracker_name(&self.tracker).device_id(device_id).send().await?;
        let p = out.position();
        Ok((p.len() == 2).then(|| Position {
            lat: p[1],
            lng: p[0],
            at: chrono::DateTime::from_timestamp(out.sample_time().secs(), 0).unwrap_or_default(),
            accuracy_m: out.accuracy().map(|a| a.horizontal()),
        }))
    }
}

#[derive(Default)]
pub struct FakeTracker {
    last: Mutex<HashMap<String, Position>>,
}

#[async_trait]
impl TrackerPort for FakeTracker {
    async fn update(&self, device_id: &str, positions: &[Position]) -> anyhow::Result<()> {
        if let Some(p) = positions.iter().max_by_key(|p| p.at) {
            self.last.lock().unwrap().insert(device_id.into(), p.clone());
        }
        Ok(())
    }

    async fn latest(&self, device_id: &str) -> anyhow::Result<Option<Position>> {
        Ok(self.last.lock().unwrap().get(device_id).cloned())
    }
}
