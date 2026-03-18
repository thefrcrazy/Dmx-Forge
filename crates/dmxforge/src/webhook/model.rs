use serde::{Serialize, Deserialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Provider {
    #[serde(rename = "github")]
    Github,
    #[serde(rename = "gitlab")]
    Gitlab,
    #[serde(rename = "gitea")]
    Gitea,
}

impl Provider {
    pub fn parse(input: &str) -> Option<Self> {
        match input {
            "github" => Some(Self::Github),
            "gitlab" => Some(Self::Gitlab),
            "gitea" | "forgejo" => Some(Self::Gitea),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Gitlab => "gitlab",
            Self::Gitea => "gitea",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AcceptedWebhook {
    pub delivery_id: Uuid,
    pub status: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnifiedEvent {
    pub provider: String,
    pub event_type: String,
    pub repository: UnifiedRepository,
    pub actor: UnifiedActor,
    pub branch: Option<String>,
    pub compare_url: Option<String>,
    pub commit_count: usize,
    pub commits: Vec<UnifiedCommit>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub url: Option<String>,
    pub timestamp: String,
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnifiedRepository {
    pub name: String,
    pub full_name: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnifiedActor {
    pub name: String,
    pub username: String,
    pub url: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnifiedCommit {
    pub id: String,
    pub short_id: String,
    pub message: String,
    pub url: Option<String>,
    pub author_name: Option<String>,
}

pub struct RouteWebhookTestRequest {
    pub provider: String,
    pub event_type: String,
    pub repository: String,
    pub branch: Option<String>,
}
