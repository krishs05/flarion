#[derive(Debug, Clone)]
pub struct Endpoint {
    pub name: String,
    pub url: String,
    pub api_key: Option<String>,
}
