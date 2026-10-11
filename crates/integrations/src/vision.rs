//! Amazon Rekognition (moderation, faces, selfie match) and Amazon Textract
//! (document queries, label text), PLAN.md §7.1, §8.3, §11.3, §11.5.

use async_trait::async_trait;
use std::collections::BTreeMap;

/// Normalized `[left, top, width, height]`.
pub type BBox = [f32; 4];

#[async_trait]
pub trait VisionPort: Send + Sync {
    async fn moderation_labels(&self, image: &[u8]) -> anyhow::Result<Vec<String>>;
    async fn faces(&self, image: &[u8]) -> anyhow::Result<Vec<BBox>>;
    /// Best similarity (0–100) of the source face in the target image.
    async fn compare_faces(&self, source: &[u8], target: &[u8]) -> anyhow::Result<Option<f32>>;
}

#[async_trait]
pub trait DocTextPort: Send + Sync {
    /// Textract `AnalyzeDocument` with Queries: alias → answer.
    async fn queries(&self, document: &[u8], queries: &[(&str, &str)]) -> anyhow::Result<BTreeMap<String, String>>;
    /// Textract `DetectDocumentText`: all lines joined.
    async fn text(&self, image: &[u8]) -> anyhow::Result<String>;
}

pub struct AwsVision {
    client: aws_sdk_rekognition::Client,
}

impl AwsVision {
    pub fn new(client: aws_sdk_rekognition::Client) -> Self {
        Self { client }
    }
}

fn rk_image(bytes: &[u8]) -> aws_sdk_rekognition::types::Image {
    aws_sdk_rekognition::types::Image::builder().bytes(aws_sdk_rekognition::primitives::Blob::new(bytes.to_vec())).build()
}

#[async_trait]
impl VisionPort for AwsVision {
    async fn moderation_labels(&self, image: &[u8]) -> anyhow::Result<Vec<String>> {
        let out = self.client.detect_moderation_labels().image(rk_image(image)).min_confidence(60.0).send().await?;
        Ok(out.moderation_labels().iter().filter_map(|l| l.name().map(str::to_owned)).collect())
    }

    async fn faces(&self, image: &[u8]) -> anyhow::Result<Vec<BBox>> {
        let out = self.client.detect_faces().image(rk_image(image)).send().await?;
        Ok(out
            .face_details()
            .iter()
            .filter_map(|f| f.bounding_box())
            .map(|b| [b.left().unwrap_or(0.0), b.top().unwrap_or(0.0), b.width().unwrap_or(0.0), b.height().unwrap_or(0.0)])
            .collect())
    }

    async fn compare_faces(&self, source: &[u8], target: &[u8]) -> anyhow::Result<Option<f32>> {
        let out = self
            .client
            .compare_faces()
            .source_image(rk_image(source))
            .target_image(rk_image(target))
            .similarity_threshold(50.0)
            .send()
            .await?;
        Ok(out.face_matches().iter().filter_map(|m| m.similarity()).reduce(f32::max))
    }
}

pub struct AwsDocText {
    client: aws_sdk_textract::Client,
}

impl AwsDocText {
    pub fn new(client: aws_sdk_textract::Client) -> Self {
        Self { client }
    }
}

fn tx_doc(bytes: &[u8]) -> aws_sdk_textract::types::Document {
    aws_sdk_textract::types::Document::builder().bytes(aws_sdk_textract::primitives::Blob::new(bytes.to_vec())).build()
}

#[async_trait]
impl DocTextPort for AwsDocText {
    async fn queries(&self, document: &[u8], queries: &[(&str, &str)]) -> anyhow::Result<BTreeMap<String, String>> {
        use aws_sdk_textract::types::{BlockType, FeatureType, QueriesConfig, Query, RelationshipType};
        let mut qc = QueriesConfig::builder();
        for (alias, text) in queries {
            qc = qc.queries(Query::builder().alias(*alias).text(*text).build()?);
        }
        let out = self
            .client
            .analyze_document()
            .document(tx_doc(document))
            .feature_types(FeatureType::Queries)
            .queries_config(qc.build()?)
            .send()
            .await?;
        let blocks = out.blocks();
        let by_id: BTreeMap<&str, &aws_sdk_textract::types::Block> = blocks.iter().filter_map(|b| b.id().map(|id| (id, b))).collect();
        let mut answers = BTreeMap::new();
        for b in blocks.iter().filter(|b| b.block_type() == Some(&BlockType::Query)) {
            let Some(alias) = b.query().and_then(|q| q.alias()) else { continue };
            for rel in b.relationships().iter().filter(|r| r.r#type() == Some(&RelationshipType::Answer)) {
                if let Some(text) = rel.ids().first().and_then(|id| by_id.get(id.as_str())).and_then(|a| a.text()) {
                    answers.insert(alias.to_owned(), text.to_owned());
                }
            }
        }
        Ok(answers)
    }

    async fn text(&self, image: &[u8]) -> anyhow::Result<String> {
        use aws_sdk_textract::types::BlockType;
        let out = self.client.detect_document_text().document(tx_doc(image)).send().await?;
        Ok(out.blocks().iter().filter(|b| b.block_type() == Some(&BlockType::Line)).filter_map(|b| b.text()).collect::<Vec<_>>().join("\n"))
    }
}

/// Local fake: no labels, no faces, no text.
pub struct FakeVision;

#[async_trait]
impl VisionPort for FakeVision {
    async fn moderation_labels(&self, _image: &[u8]) -> anyhow::Result<Vec<String>> {
        Ok(vec![])
    }
    async fn faces(&self, _image: &[u8]) -> anyhow::Result<Vec<BBox>> {
        Ok(vec![])
    }
    async fn compare_faces(&self, _s: &[u8], _t: &[u8]) -> anyhow::Result<Option<f32>> {
        Ok(None)
    }
}

pub struct FakeDocText;

#[async_trait]
impl DocTextPort for FakeDocText {
    async fn queries(&self, _d: &[u8], _q: &[(&str, &str)]) -> anyhow::Result<BTreeMap<String, String>> {
        Ok(BTreeMap::new())
    }
    async fn text(&self, _i: &[u8]) -> anyhow::Result<String> {
        Ok(String::new())
    }
}
