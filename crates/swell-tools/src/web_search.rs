//! Web search tools for external information retrieval.
//!
//! This module provides:
//! - [`WebSearchTool`] - general-purpose web search
//! - [`DomainSearchTool`] - search restricted to specific domains
//! - [`FetchPageTool`] - page fetch with content extraction and provenance
//!
//! Tools support:
//! - Permission tiers (Auto/Ask/Deny)
//! - Domain allowlists and blocklists
//! - Rate limiting

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use swell_core::traits::{Tool, ToolBehavioralHints};
use swell_core::{PermissionTier, SwellError, ToolOutput, ToolResultContent, ToolRiskLevel};
use tracing::info;

/// Rate limiter for controlling request frequency
#[derive(Debug, Clone)]
pub struct RateLimiter {
    #[allow(dead_code)]
    requests_per_minute: u32,
    #[allow(dead_code)]
    window_secs: u64,
    #[allow(dead_code)]
    domain: Option<String>,
}

impl RateLimiter {
    pub fn new(requests_per_minute: u32) -> Self {
        Self {
            requests_per_minute,
            window_secs: 60,
            domain: None,
        }
    }

    pub fn with_domain(mut self, domain: String) -> Self {
        self.domain = Some(domain);
        self
    }

    #[allow(dead_code)]
    pub fn is_allowed(&self) -> bool {
        // For MVP, rate limiting is a no-op
        // In production, this would track request timestamps
        true
    }
}

/// Configuration for web search tools
#[derive(Debug, Clone)]
pub struct WebSearchConfig {
    pub rate_limiter: RateLimiter,
    pub domain_allowlist: Vec<String>,
    pub domain_blocklist: Vec<String>,
    pub timeout_secs: u64,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            rate_limiter: RateLimiter::new(60), // 60 requests per minute
            domain_allowlist: Vec::new(),
            domain_blocklist: Vec::new(),
            timeout_secs: 30,
        }
    }
}

/// A single search result with title, URL, and snippet
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Provenance metadata for fetched content
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentProvenance {
    pub url: String,
    pub title: String,
    pub fetched_at: DateTime<Utc>,
    pub content_hash: Option<String>,
    /// Publication date extracted from the page (meta tags, schema.org, etc.)
    /// This is optional because many pages don't have this information
    #[serde(default)]
    pub publication_date: Option<DateTime<Utc>>,
}

/// Extracted content from a web page
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageContent {
    pub provenance: ContentProvenance,
    pub text: String,
    pub code_blocks: Vec<CodeBlock>,
}

/// A code block extracted from the page
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeBlock {
    pub language: Option<String>,
    pub code: String,
    pub start_line: u32,
    pub end_line: u32,
}

/// Shared HTTP client for all search tools
#[derive(Debug, Clone)]
pub struct SearchClient {
    client: Client,
    config: WebSearchConfig,
}

impl SearchClient {
    pub fn new(config: WebSearchConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .user_agent("SWELL/1.0 (autonomous-coding-engine)")
            .build()
            .expect("Failed to create HTTP client");

        Self { client, config }
    }

    pub fn with_config(config: WebSearchConfig) -> Self {
        Self::new(config)
    }

    /// Check if a domain is allowed based on allowlist/blocklist
    fn is_domain_allowed(&self, url: &str) -> bool {
        // Extract domain from URL
        let domain = if let Ok(url) = url::Url::parse(url) {
            url.host_str().unwrap_or("").to_string()
        } else {
            return true; // Allow if we can't parse
        };

        // Check blocklist first (deny takes precedence)
        for blocked in &self.config.domain_blocklist {
            if domain.contains(blocked) {
                return false;
            }
        }

        // If allowlist is empty, allow all (except blocked)
        if self.config.domain_allowlist.is_empty() {
            return true;
        }

        // Check allowlist
        for allowed in &self.config.domain_allowlist {
            if domain.contains(allowed) {
                return true;
            }
        }

        false
    }
}

/// Tool for general-purpose web search
#[derive(Debug, Clone)]
pub struct WebSearchTool {
    name: String,
    description: String,
    client: SearchClient,
    rate_limiter: RateLimiter,
}

impl WebSearchTool {
    pub fn new() -> Self {
        Self {
            name: "web_search".to_string(),
            description:
                "Search the web for information. Returns results with title, URL, and snippet."
                    .to_string(),
            client: SearchClient::with_config(WebSearchConfig::default()),
            rate_limiter: RateLimiter::new(60),
        }
    }

    pub fn with_config(config: WebSearchConfig) -> Self {
        Self {
            name: "web_search".to_string(),
            description:
                "Search the web for information. Returns results with title, URL, and snippet."
                    .to_string(),
            client: SearchClient::with_config(config.clone()),
            rate_limiter: config.rate_limiter,
        }
    }

    /// Perform the actual web search using DuckDuckGo HTML
    async fn do_search(&self, query: &str) -> Result<Vec<SearchResult>, SwellError> {
        let url = format!(
            "https://html.duckduckgo.com/html/?q={}",
            urlencoding::encode(query)
        );

        let response = self.client.client.get(&url).send().await.map_err(|e| {
            SwellError::ToolExecutionFailed(format!("Search request failed: {}", e))
        })?;

        if !response.status().is_success() {
            return Err(SwellError::ToolExecutionFailed(format!(
                "Search failed with status: {}",
                response.status()
            )));
        }

        let body = response.text().await.map_err(|e| {
            SwellError::ToolExecutionFailed(format!("Failed to read response: {}", e))
        })?;

        // Parse DuckDuckGo HTML results
        let results = self.parse_ddg_results(&body);
        Ok(results)
    }

    /// Parse DuckDuckGo HTML results
    fn parse_ddg_results(&self, html: &str) -> Vec<SearchResult> {
        let mut results = Vec::new();

        // Simple HTML parsing - look for result divs
        for line in html.lines() {
            let line = line.trim();

            // Look for result titles (they contain the snippet too in DuckDuckGo HTML)
            if line.contains("result__title") || line.contains("web-result") {
                // Extract title
                if let Some(title) = self.extract_title(line) {
                    let snippet = self.extract_snippet(line);
                    let url = self.extract_url(line);

                    if !title.is_empty() && !url.is_empty() {
                        results.push(SearchResult {
                            title,
                            url,
                            snippet,
                        });
                    }
                }
            }
        }

        // Limit results
        results.truncate(10);
        results
    }

    fn extract_title(&self, line: &str) -> Option<String> {
        // Try to find <a class="result__a" href="...">TITLE</a>
        if let Some(start) = line.find("result__a") {
            let after_title = &line[start..];
            if let Some(tag_end) = after_title.find('>') {
                let content = &after_title[tag_end + 1..];
                if let Some(end) = content.find('<') {
                    let title = content[..end].trim().to_string();
                    if !title.is_empty() {
                        return Some(self.clean_html(&title));
                    }
                }
            }
        }
        None
    }

    fn extract_snippet(&self, line: &str) -> String {
        // Try to find result snippet
        if let Some(start) = line.find("result__snippet") {
            let after_snippet = &line[start..];
            if let Some(tag_end) = after_snippet.find('>') {
                let content = &after_snippet[tag_end + 1..];
                if let Some(end) = content.find("</a>").or_else(|| content.find('<')) {
                    return self.clean_html(content[..end].trim());
                }
            }
        }
        String::new()
    }

    fn extract_url(&self, line: &str) -> String {
        // Try to find href in result link
        if let Some(href_start) = line.find("href=\"https://") {
            let after_href = &line[href_start + 6..]; // Skip "href=""
            if let Some(href_end) = after_href.find('"') {
                let url = &after_href[..href_end];
                // Skip DuckDuckGo redirect
                if url.starts_with("https://duckduckgo.com/l/?uddg=") {
                    if let Some(encoded_start) = url.find("uddg=") {
                        let encoded = &url[encoded_start + 5..];
                        if let Ok(decoded) = urlencoding::decode(encoded) {
                            return decoded.to_string();
                        }
                    }
                } else if url.starts_with("http") {
                    return url.to_string();
                }
            }
        }
        String::new()
    }

    fn clean_html(&self, text: &str) -> String {
        text.replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&nbsp;", " ")
    }
}

impl Default for WebSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> String {
        self.description.clone()
    }

    fn risk_level(&self) -> ToolRiskLevel {
        ToolRiskLevel::Read
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::Auto
    }

    fn behavioral_hints(&self) -> ToolBehavioralHints {
        ToolBehavioralHints {
            read_only_hint: true,
            destructive_hint: false,
            idempotent_hint: true,
        }
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query string"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results to return",
                    "default": 10
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<ToolOutput, SwellError> {
        #[derive(Deserialize)]
        struct Args {
            query: String,
            #[serde(default = "default_max_results")]
            max_results: usize,
        }

        fn default_max_results() -> usize {
            10
        }

        let args: Args = serde_json::from_value(arguments)
            .map_err(|e| SwellError::ToolExecutionFailed(e.to_string()))?;

        if !self.rate_limiter.is_allowed() {
            return Err(SwellError::ToolExecutionFailed(
                "Rate limit exceeded".to_string(),
            ));
        }

        info!(query = %args.query, "Performing web search");
        let results = self.do_search(&args.query).await?;

        let limited_results: Vec<_> = results.into_iter().take(args.max_results).collect();

        let response = serde_json::json!({
            "query": args.query,
            "results": limited_results,
            "count": limited_results.len()
        });

        Ok(ToolOutput {
            is_error: false,
            content: vec![ToolResultContent::Json(response)],
        })
    }
}

/// Tool for searching within specific trusted domains
#[derive(Debug, Clone)]
pub struct DomainSearchTool {
    name: String,
    description: String,
    client: SearchClient,
    trusted_domains: Vec<String>,
    rate_limiter: RateLimiter,
}

impl DomainSearchTool {
    pub fn new(trusted_domains: Vec<String>) -> Self {
        let domains_str = trusted_domains.join(", ");
        Self {
            name: "domain_search".to_string(),
            description: format!(
                "Search restricted to trusted domains: {}. Returns results with title, URL, and snippet.",
                domains_str
            ),
            client: SearchClient::with_config(WebSearchConfig::default()),
            trusted_domains,
            rate_limiter: RateLimiter::new(60),
        }
    }

    pub fn with_config(config: WebSearchConfig, trusted_domains: Vec<String>) -> Self {
        let domains_str = trusted_domains.join(", ");
        Self {
            name: "domain_search".to_string(),
            description: format!(
                "Search restricted to trusted domains: {}. Returns results with title, URL, and snippet.",
                domains_str
            ),
            client: SearchClient::with_config(config),
            trusted_domains,
            rate_limiter: RateLimiter::new(60),
        }
    }

    /// Perform domain-restricted search
    async fn do_search(&self, query: &str) -> Result<Vec<SearchResult>, SwellError> {
        // Add site: restrictions to query
        let site_query = if self.trusted_domains.is_empty() {
            query.to_string()
        } else {
            let site_restrictions: Vec<String> = self
                .trusted_domains
                .iter()
                .map(|d| format!("site:{}", d))
                .collect();
            format!("{} ({})", query, site_restrictions.join(" OR "))
        };

        let url = format!(
            "https://html.duckduckgo.com/html/?q={}",
            urlencoding::encode(&site_query)
        );

        let response = self.client.client.get(&url).send().await.map_err(|e| {
            SwellError::ToolExecutionFailed(format!("Search request failed: {}", e))
        })?;

        if !response.status().is_success() {
            return Err(SwellError::ToolExecutionFailed(format!(
                "Search failed with status: {}",
                response.status()
            )));
        }

        let body = response.text().await.map_err(|e| {
            SwellError::ToolExecutionFailed(format!("Failed to read response: {}", e))
        })?;

        // Parse and filter results
        let all_results = self.parse_ddg_results(&body);
        let filtered_results: Vec<SearchResult> = all_results
            .into_iter()
            .filter(|r| self.is_url_allowed(&r.url))
            .collect();

        Ok(filtered_results)
    }

    fn parse_ddg_results(&self, html: &str) -> Vec<SearchResult> {
        let mut results = Vec::new();

        for line in html.lines() {
            let line = line.trim();
            if line.contains("result__title") || line.contains("web-result") {
                if let Some(title) = self.extract_title(line) {
                    let snippet = self.extract_snippet(line);
                    let url = self.extract_url(line);

                    if !title.is_empty() && !url.is_empty() {
                        results.push(SearchResult {
                            title,
                            url,
                            snippet,
                        });
                    }
                }
            }
        }

        results.truncate(10);
        results
    }

    fn is_url_allowed(&self, url: &str) -> bool {
        if self.trusted_domains.is_empty() {
            return true;
        }

        if let Ok(parsed) = url::Url::parse(url) {
            if let Some(host) = parsed.host_str() {
                return self.trusted_domains.iter().any(|d| host.contains(d));
            }
        }
        false
    }

    fn extract_title(&self, line: &str) -> Option<String> {
        if let Some(start) = line.find("result__a") {
            let after_title = &line[start..];
            if let Some(tag_end) = after_title.find('>') {
                let content = &after_title[tag_end + 1..];
                if let Some(end) = content.find('<') {
                    let title = content[..end].trim().to_string();
                    if !title.is_empty() {
                        return Some(self.clean_html(&title));
                    }
                }
            }
        }
        None
    }

    fn extract_snippet(&self, line: &str) -> String {
        if let Some(start) = line.find("result__snippet") {
            let after_snippet = &line[start..];
            if let Some(tag_end) = after_snippet.find('>') {
                let content = &after_snippet[tag_end + 1..];
                if let Some(end) = content.find("</a>").or_else(|| content.find('<')) {
                    return self.clean_html(content[..end].trim());
                }
            }
        }
        String::new()
    }

    fn extract_url(&self, line: &str) -> String {
        if let Some(href_start) = line.find("href=\"https://") {
            let after_href = &line[href_start + 6..];
            if let Some(href_end) = after_href.find('"') {
                let url = &after_href[..href_end];
                if url.starts_with("https://duckduckgo.com/l/?uddg=") {
                    if let Some(encoded_start) = url.find("uddg=") {
                        let encoded = &url[encoded_start + 5..];
                        if let Ok(decoded) = urlencoding::decode(encoded) {
                            return decoded.to_string();
                        }
                    }
                } else if url.starts_with("http") {
                    return url.to_string();
                }
            }
        }
        String::new()
    }

    fn clean_html(&self, text: &str) -> String {
        text.replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&nbsp;", " ")
    }
}

#[async_trait]
impl Tool for DomainSearchTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> String {
        self.description.clone()
    }

    fn risk_level(&self) -> ToolRiskLevel {
        ToolRiskLevel::Read
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::Auto
    }

    fn behavioral_hints(&self) -> ToolBehavioralHints {
        ToolBehavioralHints {
            read_only_hint: true,
            destructive_hint: false,
            idempotent_hint: true,
        }
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query string"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results to return",
                    "default": 10
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<ToolOutput, SwellError> {
        #[derive(Deserialize)]
        struct Args {
            query: String,
            #[serde(default = "default_max_results")]
            max_results: usize,
        }

        fn default_max_results() -> usize {
            10
        }

        let args: Args = serde_json::from_value(arguments)
            .map_err(|e| SwellError::ToolExecutionFailed(e.to_string()))?;

        if !self.rate_limiter.is_allowed() {
            return Err(SwellError::ToolExecutionFailed(
                "Rate limit exceeded".to_string(),
            ));
        }

        info!(query = %args.query, domains = ?self.trusted_domains, "Performing domain-restricted search");
        let results = self.do_search(&args.query).await?;

        let limited_results: Vec<_> = results.into_iter().take(args.max_results).collect();

        let response = serde_json::json!({
            "query": args.query,
            "restricted_to_domains": self.trusted_domains,
            "results": limited_results,
            "count": limited_results.len()
        });

        Ok(ToolOutput {
            is_error: false,
            content: vec![ToolResultContent::Json(response)],
        })
    }
}

/// Tool for fetching web pages and extracting main content
#[derive(Debug, Clone)]
pub struct FetchPageTool {
    name: String,
    description: String,
    client: SearchClient,
    rate_limiter: RateLimiter,
}

impl FetchPageTool {
    pub fn new() -> Self {
        Self {
            name: "fetch_page".to_string(),
            description: "Fetch a web page and extract main content. Returns cleaned text with HTML removed, code blocks preserved, and provenance metadata (URL, title, timestamp)."
                .to_string(),
            client: SearchClient::with_config(WebSearchConfig::default()),
            rate_limiter: RateLimiter::new(30), // Lower rate limit for fetching
        }
    }

    pub fn with_config(config: WebSearchConfig) -> Self {
        Self {
            name: "fetch_page".to_string(),
            description: "Fetch a web page and extract main content. Returns cleaned text with HTML removed, code blocks preserved, and provenance metadata (URL, title, timestamp)."
                .to_string(),
            client: SearchClient::with_config(config.clone()),
            rate_limiter: config.rate_limiter,
        }
    }

    /// Fetch and extract content from a URL
    async fn do_fetch(&self, url: &str) -> Result<PageContent, SwellError> {
        // Check domain restrictions
        if !self.client.is_domain_allowed(url) {
            return Err(SwellError::ToolExecutionFailed(format!(
                "Domain not allowed: {}",
                url
            )));
        }

        let response =
            self.client.client.get(url).send().await.map_err(|e| {
                SwellError::ToolExecutionFailed(format!("Fetch request failed: {}", e))
            })?;

        if !response.status().is_success() {
            return Err(SwellError::ToolExecutionFailed(format!(
                "Fetch failed with status: {}",
                response.status()
            )));
        }

        let body = response.text().await.map_err(|e| {
            SwellError::ToolExecutionFailed(format!("Failed to read response: {}", e))
        })?;

        // Extract title from HTML
        let title = self.extract_title(&body);

        // Extract publication date from HTML
        let publication_date = self.extract_publication_date(&body);

        // Extract main content
        let (text, code_blocks) = self.extract_content(&body);

        let provenance = ContentProvenance {
            url: url.to_string(),
            title: title.clone(),
            fetched_at: Utc::now(),
            content_hash: None,
            publication_date,
        };

        Ok(PageContent {
            provenance,
            text,
            code_blocks,
        })
    }

    fn extract_title(&self, html: &str) -> String {
        // Try to find <title> tag
        if let Some(start) = html.find("<title") {
            let after_title = &html[start..];
            if let Some(tag_end) = after_title.find('>') {
                let content = &after_title[tag_end + 1..];
                if let Some(end) = content.find("</title>") {
                    return self.clean_html(content[..end].trim());
                }
            }
        }

        // Try og:title
        if let Some(start) = html.find("og:title") {
            let after_og = &html[start..];
            if let Some(content_start) = after_og.find("content=\"") {
                let content = &after_og[content_start + 9..];
                if let Some(end) = content.find('"') {
                    return self.clean_html(&content[..end]);
                }
            }
        }

        String::new()
    }

    /// Extract publication date from HTML metadata
    /// Looks for: article:published_time (Open Graph), datePublished (Schema.org), <time> elements
    fn extract_publication_date(&self, html: &str) -> Option<DateTime<Utc>> {
        // Try Open Graph article:published_time
        if let Some(start) = html.find("article:published_time") {
            let after_tag = &html[start..];
            if let Some(content_start) = after_tag.find("content=\"") {
                let content = &after_tag[content_start + 9..];
                if let Some(end) = content.find('"') {
                    let date_str = &content[..end];
                    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(date_str) {
                        return Some(dt.with_timezone(&Utc));
                    }
                }
            }
        }

        // Try datePublished (Schema.org)
        if let Some(start) = html.find("datePublished") {
            let after_tag = &html[start..];
            if let Some(content_start) = after_tag.find("content=\"") {
                let content = &after_tag[content_start + 9..];
                if let Some(end) = content.find('"') {
                    let date_str = &content[..end];
                    // Try RFC3339 first
                    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(date_str) {
                        return Some(dt.with_timezone(&Utc));
                    }
                    // Try ISO 8601 date-only
                    if let Ok(dt) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                        return Some(dt.and_hms_opt(0, 0, 0)?.and_utc());
                    }
                }
            }
        }

        // Try <time> element with datetime attribute
        if let Some(start) = html.find("<time") {
            let after_tag = &html[start..];
            if let Some(datetime_start) = after_tag.find("datetime=\"") {
                let datetime = &after_tag[datetime_start + 10..];
                if let Some(end) = datetime.find('"') {
                    let date_str = &datetime[..end];
                    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(date_str) {
                        return Some(dt.with_timezone(&Utc));
                    }
                    if let Ok(dt) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                        return Some(dt.and_hms_opt(0, 0, 0)?.and_utc());
                    }
                }
            }
        }

        None
    }

    fn extract_content(&self, html: &str) -> (String, Vec<CodeBlock>) {
        let mut code_blocks = Vec::new();

        // Remove script and style tags first
        let cleaned = self.remove_elements(
            html,
            &["script", "style", "nav", "header", "footer", "aside"],
        );

        // Extract code blocks before removing HTML
        let mut line_num = 0u32;
        for line in cleaned.lines() {
            line_num += 1;

            // Check for code blocks (pre/code tags)
            if line.contains("<pre") || line.contains("<code") {
                if let Some(code) = self.extract_code_block(line, line_num) {
                    code_blocks.push(code);
                }
            }
        }

        // Remove all HTML tags
        let text = self.html_to_text(&cleaned);

        // Clean up whitespace
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");

        (text, code_blocks)
    }

    fn extract_code_block(&self, line: &str, start_line: u32) -> Option<CodeBlock> {
        // Simple code block extraction
        let code = if let Some(pre_start) = line.find("<pre") {
            let after_pre = &line[pre_start..];
            // Find content between tags
            let content = after_pre.split(|c| ['>', '<'].contains(&c)).nth(1)?.trim();
            if let Some(end_idx) = content.find("</pre>") {
                content[..end_idx].to_string()
            } else {
                content.to_string()
            }
        } else if let Some(code_start) = line.find("<code") {
            let after_code = &line[code_start..];
            let content = after_code.split(|c| ['>', '<'].contains(&c)).nth(1)?.trim();
            if let Some(end_idx) = content.find("</code>") {
                content[..end_idx].to_string()
            } else {
                content.to_string()
            }
        } else {
            return None;
        };

        if code.trim().is_empty() {
            return None;
        }

        // Try to detect language from class
        let language = self.detect_language(line);

        Some(CodeBlock {
            language,
            code: self.clean_html(&code),
            start_line,
            end_line: start_line + code.lines().count() as u32,
        })
    }

    fn detect_language(&self, line: &str) -> Option<String> {
        // Check for language classes like "language-rust", "lang-python"
        if let Some(class_pos) = line.find("class=\"") {
            let after_class = &line[class_pos + 7..];
            if let Some(class_end) = after_class.find('"') {
                let classes = &after_class[..class_end];
                for lang in &[
                    "rust",
                    "python",
                    "javascript",
                    "typescript",
                    "java",
                    "go",
                    "c",
                    "cpp",
                    "ruby",
                    "bash",
                    "sh",
                    "json",
                    "yaml",
                    "toml",
                    "sql",
                    "html",
                    "css",
                ] {
                    if classes.contains(lang) {
                        return Some(lang.to_string());
                    }
                }
            }
        }
        None
    }

    fn remove_elements(&self, html: &str, elements: &[&str]) -> String {
        let mut result = html.to_string();
        for element in elements {
            // Remove opening and closing tags
            let open_pattern = format!("<{}", element);
            let close_pattern = format!("</{}>", element);

            let mut iter_count = 0usize;
            while result.contains(&open_pattern) || result.contains(&close_pattern) {
                iter_count += 1;
                if iter_count > 10_000_000 {
                    tracing::error!(
                        "web_search: remove_elements exceeded MAX_ITER for element '{}'; aborting",
                        element
                    );
                    break;
                }
                // Find and remove opening tags with content
                if let Some(start) = result.find(&open_pattern) {
                    let after_open = &result[start..];
                    if let Some(end) = after_open.find('>') {
                        let after_tag = &after_open[end + 1..];
                        // Find closing tag
                        let close_search = format!("</{}>", element);
                        if let Some(close_start) = after_tag.find(&close_search) {
                            result = format!(
                                "{}{}",
                                &result[..start],
                                &after_tag[close_start + close_search.len()..]
                            );
                        } else if let Some(next_open) = after_tag.find(&open_pattern) {
                            result = format!("{}{}", &result[..start], &after_tag[next_open..]);
                        } else {
                            result = result[..start].to_string();
                            break;
                        }
                    }
                } else {
                    break;
                }
            }
        }
        result
    }

    fn html_to_text(&self, html: &str) -> String {
        let mut text = String::new();
        let mut in_tag = false;
        let mut in_entity = false;
        let mut entity = String::new();

        for ch in html.chars() {
            match ch {
                '<' => in_tag = true,
                '>' if in_tag => {
                    in_tag = false;
                    text.push(' ');
                }
                '&' if !in_tag => {
                    in_entity = true;
                    entity.clear();
                    entity.push(ch);
                }
                ';' if in_entity => {
                    entity.push(ch);
                    in_entity = false;
                    text.push(self.decode_entity(&entity));
                    entity.clear();
                }
                _ if in_entity => entity.push(ch),
                _ if !in_tag && !in_entity => text.push(ch),
                _ => {}
            }
        }

        text
    }

    fn decode_entity(&self, entity: &str) -> char {
        match entity {
            "&amp;" => '&',
            "&lt;" => '<',
            "&gt;" => '>',
            "&quot;" => '"',
            "&#39;" | "&apos;" => '\'',
            "&nbsp;" => ' ',
            "&copy;" => '©',
            "&reg;" => '®',
            "&trade;" => '™',
            _ => {
                // Try numeric entities
                if entity.starts_with("&#") {
                    if let Ok(code) = entity[2..entity.len() - 1].parse::<u32>() {
                        if let Some(ch) = char::from_u32(code) {
                            return ch;
                        }
                    }
                }
                ' '
            }
        }
    }

    fn clean_html(&self, text: &str) -> String {
        text.replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&nbsp;", " ")
    }
}

impl Default for FetchPageTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for FetchPageTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> String {
        self.description.clone()
    }

    fn risk_level(&self) -> ToolRiskLevel {
        ToolRiskLevel::Read
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::Auto
    }

    fn behavioral_hints(&self) -> ToolBehavioralHints {
        ToolBehavioralHints {
            read_only_hint: true,
            destructive_hint: false,
            idempotent_hint: true,
        }
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "URL of the page to fetch"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<ToolOutput, SwellError> {
        #[derive(Deserialize)]
        struct Args {
            url: String,
        }

        let args: Args = serde_json::from_value(arguments)
            .map_err(|e| SwellError::ToolExecutionFailed(e.to_string()))?;

        if !self.rate_limiter.is_allowed() {
            return Err(SwellError::ToolExecutionFailed(
                "Rate limit exceeded".to_string(),
            ));
        }

        info!(url = %args.url, "Fetching web page");
        let content = self.do_fetch(&args.url).await?;

        let response = serde_json::json!({
            "provenance": content.provenance,
            "text": content.text,
            "code_blocks": content.code_blocks,
            "code_block_count": content.code_blocks.len()
        });

        Ok(ToolOutput {
            is_error: false,
            content: vec![ToolResultContent::Json(response)],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_web_search_tool_creation() {
        let tool = WebSearchTool::new();
        assert_eq!(tool.name(), "web_search");
        assert!(!tool.description().is_empty());
        assert_eq!(tool.risk_level(), ToolRiskLevel::Read);
        assert_eq!(tool.permission_tier(), PermissionTier::Auto);
    }

    #[tokio::test]
    async fn test_domain_search_tool_creation() {
        let domains = vec!["docs.rs".to_string(), "github.com".to_string()];
        let tool = DomainSearchTool::new(domains.clone());
        assert_eq!(tool.name(), "domain_search");
        assert!(tool.description().contains("docs.rs"));
    }

    #[tokio::test]
    async fn test_fetch_page_tool_creation() {
        let tool = FetchPageTool::new();
        assert_eq!(tool.name(), "fetch_page");
        assert!(!tool.description().is_empty());
    }

    #[tokio::test]
    async fn test_web_search_input_schema() {
        let tool = WebSearchTool::new();
        let schema = tool.input_schema();

        assert!(schema.get("properties").is_some());
        let props = schema.get("properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("query"));
        assert!(props.contains_key("max_results"));
    }

    #[tokio::test]
    async fn test_fetch_page_input_schema() {
        let tool = FetchPageTool::new();
        let schema = tool.input_schema();

        let props = schema.get("properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("url"));
    }

    #[tokio::test]
    async fn test_behavioral_hints_web_search() {
        let tool = WebSearchTool::new();
        let hints = tool.behavioral_hints();
        assert!(hints.read_only_hint);
        assert!(!hints.destructive_hint);
        assert!(hints.idempotent_hint);
    }

    #[tokio::test]
    async fn test_behavioral_hints_fetch_page() {
        let tool = FetchPageTool::new();
        let hints = tool.behavioral_hints();
        assert!(hints.read_only_hint);
        assert!(!hints.destructive_hint);
        assert!(hints.idempotent_hint);
    }

    #[test]
    fn test_clean_html_entities() {
        let tool = WebSearchTool::new();
        assert_eq!(tool.clean_html("Hello &amp; World"), "Hello & World");
        assert_eq!(tool.clean_html("&lt;code&gt;"), "<code>");
        assert_eq!(tool.clean_html("&quot;quoted&quot;"), "\"quoted\"");
    }

    #[test]
    fn test_rate_limiter_allows() {
        let limiter = RateLimiter::new(60);
        assert!(limiter.is_allowed());
    }

    #[test]
    fn test_web_search_config_default() {
        let config = WebSearchConfig::default();
        assert_eq!(config.rate_limiter.requests_per_minute, 60);
        assert!(config.domain_allowlist.is_empty());
        assert!(config.domain_blocklist.is_empty());
    }

    #[test]
    fn test_search_result_serialization() {
        let result = SearchResult {
            title: "Test Title".to_string(),
            url: "https://example.com".to_string(),
            snippet: "Test snippet".to_string(),
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("Test Title"));
        assert!(json.contains("https://example.com"));
    }

    #[test]
    fn test_content_provenance_serialization() {
        let provenance = ContentProvenance {
            url: "https://example.com".to_string(),
            title: "Example".to_string(),
            fetched_at: Utc::now(),
            content_hash: None,
            publication_date: None,
        };

        let json = serde_json::to_string(&provenance).unwrap();
        assert!(json.contains("https://example.com"));
        assert!(json.contains("Example"));
    }

    #[test]
    fn test_code_block_serialization() {
        let block = CodeBlock {
            language: Some("rust".to_string()),
            code: "fn main() {}".to_string(),
            start_line: 1,
            end_line: 2,
        };

        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains("rust"));
        assert!(json.contains("fn main()"));
    }
}
