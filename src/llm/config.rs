use serde::{Deserialize, Serialize};
use reqwest::Client;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMConfig {
    pub model_type: String,
    pub api_url: String,
    pub api_key: Option<String>,
    pub context_lines: usize,
}

#[derive(Debug, Deserialize)]
struct ModelResponse {
    data: Vec<Model>,
}

#[derive(Debug, Deserialize)]
struct Model {
    id: String,
    name: Option<String>,
}

impl Default for LLMConfig {
    fn default() -> Self {
        Self {
            model_type: "lmstudio".to_string(),
            api_url: "http://localhost:1234/v1".to_string(),
            api_key: None,
            context_lines: 5,
        }
    }
}

impl LLMConfig {
    pub fn new(api_url: String, api_key: Option<String>, model_type: String) -> Self {
        Self {
            api_url,
            api_key,
            model_type,
            context_lines: 10, // Default value
        }
    }
}

pub async fn fetch_available_models(api_url: &str, api_key: Option<&str>) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let client = Client::new();
    let mut request = client.get(&format!("{}/models", api_url));
    
    if let Some(key) = api_key {
        request = request.header("Authorization", format!("Bearer {}", key));
    }
    
    let response = request.send().await?;
    
    if !response.status().is_success() {
        return Err(format!("Failed to fetch models: {}", response.status()).into());
    }
    
    let model_response: ModelResponse = response.json().await?;
    
    Ok(model_response.data
        .into_iter()
        .map(|model| model.name.unwrap_or(model.id))
        .collect())
} 