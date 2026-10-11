//! Address search via Amazon Location Places v2 (PLAN.md §5.3). Clients call
//! these through the API (cached, rate-limited, no keys in clients).

use async_trait::async_trait;
use kabados_domain::geo::LatLng;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Place {
    pub place_id: Option<String>,
    pub title: String,
    pub address: String,
    pub position: Option<LatLng>,
    pub city: Option<String>,
}

#[async_trait]
pub trait PlacesPort: Send + Sync {
    async fn autocomplete(&self, q: &str, bias: Option<LatLng>) -> anyhow::Result<Vec<Place>>;
    async fn geocode(&self, q: &str) -> anyhow::Result<Vec<Place>>;
    async fn reverse(&self, p: LatLng) -> anyhow::Result<Option<Place>>;
}

pub struct AwsPlaces {
    client: aws_sdk_geoplaces::Client,
}

impl AwsPlaces {
    pub fn new(client: aws_sdk_geoplaces::Client) -> Self {
        Self { client }
    }
}

fn pos(v: &[f64]) -> Option<LatLng> {
    (v.len() == 2).then(|| LatLng { lat: v[1], lng: v[0] })
}

fn city_of(a: Option<&aws_sdk_geoplaces::types::Address>) -> Option<String> {
    a.and_then(|a| a.locality()).map(|c| c.to_lowercase())
}

#[async_trait]
impl PlacesPort for AwsPlaces {
    async fn autocomplete(&self, q: &str, bias: Option<LatLng>) -> anyhow::Result<Vec<Place>> {
        use aws_sdk_geoplaces::types::AutocompleteFilter;
        let mut req =
            self.client.autocomplete().query_text(q).max_results(6).filter(AutocompleteFilter::builder().include_countries("IND").build());
        if let Some(b) = bias {
            req = req.bias_position(b.lng).bias_position(b.lat);
        }
        let out = req.send().await?;
        Ok(out
            .result_items()
            .iter()
            .map(|r| Place {
                place_id: Some(r.place_id().to_owned()),
                title: r.title().to_owned(),
                address: r.address().and_then(|a| a.label()).unwrap_or_default().to_owned(),
                position: None,
                city: city_of(r.address()),
            })
            .collect())
    }

    async fn geocode(&self, q: &str) -> anyhow::Result<Vec<Place>> {
        use aws_sdk_geoplaces::types::GeocodeFilter;
        let out = self
            .client
            .geocode()
            .query_text(q)
            .max_results(5)
            .filter(GeocodeFilter::builder().include_countries("IND").build())
            .send()
            .await?;
        Ok(out
            .result_items()
            .iter()
            .map(|r| Place {
                place_id: Some(r.place_id().to_owned()),
                title: r.title().to_owned(),
                address: r.address().and_then(|a| a.label()).unwrap_or_default().to_owned(),
                position: pos(r.position()),
                city: city_of(r.address()),
            })
            .collect())
    }

    async fn reverse(&self, p: LatLng) -> anyhow::Result<Option<Place>> {
        let out = self.client.reverse_geocode().query_position(p.lng).query_position(p.lat).max_results(1).send().await?;
        Ok(out.result_items().first().map(|r| Place {
            place_id: Some(r.place_id().to_owned()),
            title: r.title().to_owned(),
            address: r.address().and_then(|a| a.label()).unwrap_or_default().to_owned(),
            position: pos(r.position()).or(Some(p)),
            city: city_of(r.address()),
        }))
    }
}

/// Small offline gazetteer for local development.
pub struct FakePlaces;

const GAZETTEER: &[(&str, &str, f64, f64)] = &[
    ("Connaught Place", "delhi", 28.6315, 77.2167),
    ("Lajpat Nagar", "delhi", 28.5677, 77.2433),
    ("Dwarka Sector 10", "delhi", 28.5823, 77.0500),
    ("Rohini Sector 9", "delhi", 28.7158, 77.1118),
    ("Saket", "delhi", 28.5245, 77.2066),
    ("Indiranagar", "bengaluru", 12.9784, 77.6408),
    ("Koramangala", "bengaluru", 12.9352, 77.6245),
    ("Whitefield", "bengaluru", 12.9698, 77.7500),
    ("Andheri West", "mumbai", 19.1364, 72.8296),
    ("Bandra West", "mumbai", 19.0596, 72.8295),
    ("Powai", "mumbai", 19.1176, 72.9060),
    ("Salt Lake Sector V", "kolkata", 22.5726, 88.4339),
    ("T. Nagar", "chennai", 13.0418, 80.2341),
    ("Hitech City", "hyderabad", 17.4435, 78.3772),
    ("Kothrud", "pune", 18.5074, 73.8077),
];

fn fake(name: &str, city: &str, lat: f64, lng: f64) -> Place {
    Place {
        place_id: Some(format!("fake:{}", name.to_lowercase().replace(' ', "-"))),
        title: name.into(),
        address: format!("{name}, {}", capitalize(city)),
        position: Some(LatLng { lat, lng }),
        city: Some(city.into()),
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

#[async_trait]
impl PlacesPort for FakePlaces {
    async fn autocomplete(&self, q: &str, _bias: Option<LatLng>) -> anyhow::Result<Vec<Place>> {
        let q = q.to_lowercase();
        Ok(GAZETTEER
            .iter()
            .filter(|(n, c, ..)| n.to_lowercase().contains(&q) || c.contains(&q))
            .take(6)
            .map(|(n, c, la, lo)| fake(n, c, *la, *lo))
            .collect())
    }

    async fn geocode(&self, q: &str) -> anyhow::Result<Vec<Place>> {
        self.autocomplete(q, None).await
    }

    async fn reverse(&self, p: LatLng) -> anyhow::Result<Option<Place>> {
        let nearest = GAZETTEER.iter().min_by(|a, b| {
            let da = LatLng { lat: a.2, lng: a.3 }.distance_m(&p);
            let db = LatLng { lat: b.2, lng: b.3 }.distance_m(&p);
            da.total_cmp(&db)
        });
        Ok(nearest.map(|(n, c, ..)| Place { position: Some(p), ..fake(n, c, p.lat, p.lng) }))
    }
}
