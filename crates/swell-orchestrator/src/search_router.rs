//! Search router for classifying and routing queries to appropriate search strategies.
//!
//! This module provides:
//! - [`SearchDepth`] - classification for quick vs deep search
//! - [`SearchRouter`] - main router with query analysis, rewriting, and decomposition
//!
//! ## Search Depth Classification
//!
//! - **Quick**: Simple lookups, exact error messages, single sources
//! - **Deep**: Multi-part research, documentation, multiple sources
//!
//! ## Query Processing Pipeline
//!
//! 1. Classify depth (Quick/Deep) based on query complexity
//! 2. Rewrite query for precision (version-aware, disambiguation)
//! 3. Restrict domains for official docs when appropriate
//! 4. Decompose complex queries into sub-queries

use serde::{Deserialize, Serialize};

/// Search depth classification for routing queries
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchDepth {
    /// Quick search: simple lookups, exact errors, single sources
    Quick,
    /// Deep research: multi-part research, documentation, multiple sources
    Deep,
}

/// A sub-query decomposed from a complex query
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubQuery {
    /// The decomposed query string
    pub query: String,
    /// Why this sub-query was created
    pub rationale: String,
    /// Priority order (lower = more important)
    pub priority: u8,
}

/// Official documentation domains for common languages and tools
const OFFICIAL_DOC_DOMAINS: &[&str] = &[
    // Rust
    "doc.rust-lang.org",
    "rust-lang.org",
    "crates.io",
    // JavaScript/TypeScript
    "developer.mozilla.org",
    "nodejs.org",
    "npmjs.com",
    "typescriptlang.org",
    // Python
    "docs.python.org",
    "pypi.org",
    // Go
    "go.dev",
    "pkg.go.dev",
    // Java
    "docs.oracle.com",
    "maven.apache.org",
    // C/C++
    "en.cppreference.com",
    "cplusplus.com",
    // Misc
    "docker.com",
    "kubernetes.io",
    "git-scm.com",
];

/// Search domain configuration for official documentation
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchDomains {
    /// Domains to restrict search to (empty = no restriction)
    pub restricted: Vec<String>,
    /// Whether to use official doc domains only
    pub official_docs_only: bool,
}

impl SearchDomains {
    /// Create with official docs restriction for a given language/tool
    pub fn for_language(language: &str) -> Self {
        let language_lower = language.to_lowercase();
        let domains: Vec<String> = match language_lower.as_str() {
            "rust" | "rs" => vec!["doc.rust-lang.org".to_string(), "crates.io".to_string()],
            "javascript" | "js" | "typescript" | "ts" => vec![
                "developer.mozilla.org".to_string(),
                "nodejs.org".to_string(),
                "typescriptlang.org".to_string(),
            ],
            "python" | "py" => vec!["docs.python.org".to_string(), "pypi.org".to_string()],
            "go" | "golang" => vec!["go.dev".to_string(), "pkg.go.dev".to_string()],
            "java" => vec!["docs.oracle.com".to_string()],
            "c" | "c++" | "cpp" => vec![
                "en.cppreference.com".to_string(),
                "cplusplus.com".to_string(),
            ],
            _ => vec![],
        };

        let is_empty = domains.is_empty();
        Self {
            restricted: domains,
            official_docs_only: !is_empty,
        }
    }

    /// Get all official doc domains
    pub fn official_domains() -> Vec<String> {
        OFFICIAL_DOC_DOMAINS.iter().map(|s| s.to_string()).collect()
    }
}

/// Version patterns for detection and rewriting
const VERSION_PATTERNS: &[&str] = &[
    r"\d+\.\d+\.\d+", // semver: 1.2.3
    r"\d+\.\d+",      // short version: 1.2 or 1.0
    r"v\d+\.\d+",     // v-prefixed: v1.2
    r"v\d+",          // v-prefixed simple: v1
    r"version\s+\d+", // "version 1"
];

/// Query rewrite result with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewrittenQuery {
    /// The rewritten query string
    pub query: String,
    /// Whether version was detected and handled
    pub version_detected: bool,
    /// The detected version (if any)
    pub detected_version: Option<String>,
    /// Language/tool detected (if any)
    pub detected_language: Option<String>,
}

/// Search routing decision with all metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingDecision {
    /// The original query
    pub original_query: String,
    /// The search depth classification
    pub depth: SearchDepth,
    /// The rewritten query
    pub rewritten_query: RewrittenQuery,
    /// Domain restrictions (if any)
    pub domains: SearchDomains,
    /// Sub-queries for complex research (if any)
    pub sub_queries: Vec<SubQuery>,
    /// Confidence in the routing decision
    pub confidence: f64,
}

/// Main search router for classifying and routing queries
#[derive(Debug, Clone)]
pub struct SearchRouter {
    /// Quick search indicators (error messages, simple lookups)
    quick_indicators: Vec<String>,
    /// Deep search indicators (multi-part, documentation)
    deep_indicators: Vec<String>,
    /// Minimum length for deep classification when no indicators match
    min_deep_length: usize,
}

impl Default for SearchRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchRouter {
    /// Create a new SearchRouter with default configuration
    pub fn new() -> Self {
        Self {
            quick_indicators: vec![
                "error:".to_string(),
                "exception:".to_string(),
                "failed:".to_string(),
                "undefined".to_string(),
                "not found".to_string(),
                "cannot find".to_string(),
                "syntax error".to_string(),
                "undefined symbol".to_string(),
                "segmentation fault".to_string(),
                "panic:".to_string(),
                "assertion failed".to_string(),
            ],
            deep_indicators: vec![
                "how to".to_string(),
                "best practices".to_string(),
                "tutorial".to_string(),
                "guide".to_string(),
                "documentation".to_string(),
                "explain".to_string(),
                "compare".to_string(),
                "difference between".to_string(),
                " pros and cons".to_string(),
                "architecture".to_string(),
                "design pattern".to_string(),
                "implement".to_string(),
                "integrate".to_string(),
            ],
            min_deep_length: 50,
        }
    }

    /// Classify the search depth based on the query
    pub fn classify_depth(&self, query: &str) -> SearchDepth {
        let query_lower = query.to_lowercase();

        // Check if query contains quick search indicators
        let is_quick = self
            .quick_indicators
            .iter()
            .any(|indicator| query_lower.contains(&indicator.to_lowercase()));

        if is_quick {
            return SearchDepth::Quick;
        }

        // Check for deep search indicators
        let is_deep = self
            .deep_indicators
            .iter()
            .any(|indicator| query_lower.contains(&indicator.to_lowercase()));

        // Also classify as deep if query is long (complex research)
        if is_deep || query.len() >= self.min_deep_length {
            SearchDepth::Deep
        } else {
            // Short queries without indicators default to quick
            SearchDepth::Quick
        }
    }

    /// Detect version in query and return (cleaned_query, version)
    fn detect_version(&self, query: &str) -> (String, Option<String>) {
        use regex::Regex;

        let query_lower = query.to_lowercase();

        for pattern in VERSION_PATTERNS {
            if let Ok(re) = Regex::new(pattern) {
                if let Some(m) = re.find(&query_lower) {
                    let version = m.as_str().to_string();
                    // Remove version from query to get cleaner search
                    let cleaned = query_lower.replace(&version, "").trim().to_string();
                    return (cleaned, Some(version));
                }
            }
        }

        (query.to_string(), None)
    }

    /// Detect language or tool from query
    fn detect_language(&self, query: &str) -> Option<String> {
        let query_lower = query.to_lowercase();

        let languages = [
            ("rust", "Rust"),
            ("rustlang", "Rust"),
            ("python", "Python"),
            ("javascript", "JavaScript"),
            ("typescript", "TypeScript"),
            ("js", "JavaScript"),
            ("ts", "TypeScript"),
            ("go", "Go"),
            ("golang", "Go"),
            ("java", "Java"),
            ("c++", "C++"),
            ("cpp", "C++"),
            ("node", "Node.js"),
            ("nodejs", "Node.js"),
            ("react", "React"),
            ("vue", "Vue"),
            ("angular", "Angular"),
            ("django", "Django"),
            ("flask", "Flask"),
            ("fastapi", "FastAPI"),
            ("react native", "React Native"),
            ("docker", "Docker"),
            ("kubernetes", "Kubernetes"),
            ("k8s", "Kubernetes"),
            ("git", "Git"),
            ("sql", "SQL"),
            ("postgres", "PostgreSQL"),
            ("mysql", "MySQL"),
            ("mongodb", "MongoDB"),
            ("redis", "Redis"),
        ];

        for (keyword, name) in languages {
            if query_lower.contains(keyword) {
                return Some(name.to_string());
            }
        }

        None
    }

    /// Rewrite query for better search results (version-aware, precise)
    pub fn rewrite_query(&self, query: &str) -> RewrittenQuery {
        // Detect and handle version
        let (query_without_version, detected_version) = self.detect_version(query);

        // Detect language/tool
        let detected_language = self.detect_language(&query_without_version);

        // Clean up the query
        let mut rewritten = query_without_version
            .replace("  ", " ")
            .replace("??", "?")
            .trim()
            .to_string();

        // If version was detected, re-add it in a cleaner format
        if let Some(ref version) = detected_version {
            // Check if version is already well-formatted
            if !version.starts_with("version") && !version.starts_with("v") {
                rewritten = format!("{} {}", rewritten, version);
            }
        }

        // Capitalize language names for better search
        if let Some(ref lang) = detected_language {
            // Replace lowercase variants with proper case
            let lang_lower = lang.to_lowercase();
            if rewritten.to_lowercase().contains(&lang_lower) {
                rewritten = rewritten.to_lowercase().replace(&lang_lower, lang);
            }
        }

        RewrittenQuery {
            query: rewritten,
            version_detected: detected_version.is_some(),
            detected_version,
            detected_language,
        }
    }

    /// Get domain restrictions for official documentation searches
    pub fn get_official_doc_domains(&self, query: &str) -> SearchDomains {
        // First try to detect language from query
        if let Some(lang) = self.detect_language(query) {
            return SearchDomains::for_language(&lang);
        }

        // Check for framework/library indicators
        let query_lower = query.to_lowercase();

        if query_lower.contains("async") && query_lower.contains("rust") {
            return SearchDomains::for_language("rust");
        }
        if query_lower.contains("tokio") || query_lower.contains("async-std") {
            return SearchDomains::for_language("rust");
        }
        if query_lower.contains("reqwest") || query_lower.contains("hyper") {
            return SearchDomains::for_language("rust");
        }
        if query_lower.contains("express") {
            return SearchDomains::for_language("javascript");
        }
        if query_lower.contains("fastapi") || query_lower.contains("uvicorn") {
            return SearchDomains::for_language("python");
        }

        // Default: no restriction
        SearchDomains::default()
    }

    /// Decompose a complex query into sub-queries for parallel research
    pub fn decompose_into_subqueries(&self, query: &str) -> Vec<SubQuery> {
        let query_lower = query.to_lowercase();
        let mut sub_queries = Vec::new();

        // Check for multi-part research patterns
        let has_comparison = query_lower.contains(" vs ")
            || query_lower.contains(" versus ")
            || query_lower.contains(" compared to ")
            || query_lower.contains(" difference between ");

        let has_integration = query_lower.contains(" integrate ")
            || query_lower.contains(" integration ")
            || query_lower.contains(" combining ");

        let has_implementation = query_lower.contains(" implement ")
            || query_lower.contains(" building ")
            || query_lower.contains(" creating ");

        let has_best_practices = query_lower.contains(" best practice")
            || query_lower.contains(" recommended ")
            || query_lower.contains(" should ");

        // Decompose based on patterns
        if has_comparison {
            // Split by " vs " or similar
            if let Some(pos) = query_lower.find(" vs ") {
                let (part1, rest) = query.split_at(pos);
                let rest = rest.strip_prefix(" vs ").unwrap_or(rest).trim();

                if !part1.trim().is_empty() && !rest.is_empty() {
                    sub_queries.push(SubQuery {
                        query: format!("What is {}", part1.trim()),
                        rationale: "First part of comparison".to_string(),
                        priority: 1,
                    });
                    sub_queries.push(SubQuery {
                        query: format!("What is {}", rest),
                        rationale: "Second part of comparison".to_string(),
                        priority: 2,
                    });
                    sub_queries.push(SubQuery {
                        query: format!("Comparison: {} vs {}", part1.trim(), rest),
                        rationale: "Overall comparison".to_string(),
                        priority: 0,
                    });
                }
            } else if let Some(pos) = query_lower.find(" versus ") {
                let (part1, rest) = query.split_at(pos);
                let rest = rest.strip_prefix(" versus ").unwrap_or(rest).trim();

                if !part1.trim().is_empty() && !rest.is_empty() {
                    sub_queries.push(SubQuery {
                        query: format!("What is {}", part1.trim()),
                        rationale: "First part of comparison".to_string(),
                        priority: 1,
                    });
                    sub_queries.push(SubQuery {
                        query: format!("What is {}", rest),
                        rationale: "Second part of comparison".to_string(),
                        priority: 2,
                    });
                    sub_queries.push(SubQuery {
                        query: format!("Comparison: {} versus {}", part1.trim(), rest),
                        rationale: "Overall comparison".to_string(),
                        priority: 0,
                    });
                }
            }
        }

        if has_integration {
            // Break into setup + usage
            let parts: Vec<&str> = query.splitn(3, ' ').collect();
            if parts.len() >= 2 {
                sub_queries.push(SubQuery {
                    query: format!("Getting started with {}", parts[0]),
                    rationale: "Setup/installation".to_string(),
                    priority: 1,
                });
                sub_queries.push(SubQuery {
                    query: query.to_string(),
                    rationale: "Integration usage".to_string(),
                    priority: 0,
                });
            }
        }

        if has_implementation && sub_queries.is_empty() {
            // Break into concept + example
            if query_lower.contains("how to") {
                if let Some(pos) = query_lower.find("how to") {
                    let after_how_to = &query[pos + 6..].trim();
                    sub_queries.push(SubQuery {
                        query: after_how_to.to_string(),
                        rationale: "Implementation goal".to_string(),
                        priority: 0,
                    });
                }
            }
            if let Some(pos) = query_lower.find("example") {
                let after_example = &query[pos + 7..].trim();
                if !after_example.is_empty() {
                    sub_queries.push(SubQuery {
                        query: format!("Code example: {}", after_example),
                        rationale: "Code example request".to_string(),
                        priority: 1,
                    });
                }
            }
            if sub_queries.is_empty() {
                // General implementation query
                sub_queries.push(SubQuery {
                    query: query.to_string(),
                    rationale: "Implementation research".to_string(),
                    priority: 0,
                });
            }
        }

        if has_best_practices {
            // Break into problem + best practices
            sub_queries.push(SubQuery {
                query: query.to_string(),
                rationale: "Best practices research".to_string(),
                priority: 0,
            });
        }

        // If no decomposition pattern matched but query is long, do simple chunking
        if sub_queries.is_empty() && query.len() > 100 {
            // Split into key aspects
            let words: Vec<&str> = query.split_whitespace().collect();
            let midpoint = words.len() / 2;

            let first_half = words[..midpoint].join(" ");
            let second_half = words[midpoint..].join(" ");

            if !first_half.is_empty() {
                sub_queries.push(SubQuery {
                    query: first_half,
                    rationale: "First aspect of complex query".to_string(),
                    priority: 1,
                });
            }
            if !second_half.is_empty() {
                sub_queries.push(SubQuery {
                    query: second_half,
                    rationale: "Second aspect of complex query".to_string(),
                    priority: 2,
                });
            }
        }

        // If still no sub-queries, return original as single query
        if sub_queries.is_empty() {
            sub_queries.push(SubQuery {
                query: query.to_string(),
                rationale: "Single query (no decomposition needed)".to_string(),
                priority: 0,
            });
        }

        // Sort by priority
        sub_queries.sort_by_key(|q| q.priority);

        sub_queries
    }

    /// Make a complete routing decision for a query
    pub fn route(&self, query: &str) -> RoutingDecision {
        let depth = self.classify_depth(query);
        let rewritten_query = self.rewrite_query(query);
        let domains = self.get_official_doc_domains(query);

        // Only decompose deep queries
        let sub_queries = if depth == SearchDepth::Deep {
            self.decompose_into_subqueries(query)
        } else {
            vec![]
        };

        // Calculate confidence
        let confidence = if depth == SearchDepth::Quick {
            // High confidence for error messages
            if self
                .quick_indicators
                .iter()
                .any(|i| query.to_lowercase().contains(&i.to_lowercase()))
            {
                0.95
            } else {
                0.7
            }
        } else {
            // Lower confidence for deep classification
            0.75
        };

        RoutingDecision {
            original_query: query.to_string(),
            depth,
            rewritten_query,
            domains,
            sub_queries,
            confidence,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_router_classify_quick_error() {
        let router = SearchRouter::new();
        let query = "error: cannot find trait `Foo` in this scope";
        assert_eq!(router.classify_depth(query), SearchDepth::Quick);
    }

    #[test]
    fn test_search_router_classify_quick_short() {
        let router = SearchRouter::new();
        let query = "what is Rust async trait";
        assert_eq!(router.classify_depth(query), SearchDepth::Quick);
    }

    #[test]
    fn test_search_router_classify_deep_complex() {
        let router = SearchRouter::new();
        let query = "How to implement a custom async runtime in Rust with work stealing queue and io_uring integration";
        assert_eq!(router.classify_depth(query), SearchDepth::Deep);
    }

    #[test]
    fn test_search_router_classify_deep_with_indicator() {
        let router = SearchRouter::new();
        let query = "How to integrate React with Redux for state management";
        assert_eq!(router.classify_depth(query), SearchDepth::Deep);
    }

    #[test]
    fn test_rewrite_query_version_detection() {
        let router = SearchRouter::new();
        let result = router.rewrite_query("tokio 1.0 async runtime");
        assert!(result.version_detected);
        assert_eq!(result.detected_version, Some("1.0".to_string()));
    }

    #[test]
    fn test_rewrite_query_language_detection() {
        let router = SearchRouter::new();
        let result = router.rewrite_query("async trait in rust");
        assert_eq!(result.detected_language, Some("Rust".to_string()));
    }

    #[test]
    fn test_official_doc_domains_rust() {
        let domains = SearchDomains::for_language("rust");
        assert!(!domains.restricted.is_empty());
        assert!(domains
            .restricted
            .iter()
            .any(|d| d.contains("rust-lang.org")));
    }

    #[test]
    fn test_official_doc_domains_python() {
        let domains = SearchDomains::for_language("python");
        assert!(!domains.restricted.is_empty());
        assert!(domains.restricted.iter().any(|d| d.contains("python.org")));
    }

    #[test]
    fn test_decompose_comparison_query() {
        let router = SearchRouter::new();
        let subqueries = router.decompose_into_subqueries("React vs Vue for frontend development");
        assert!(!subqueries.is_empty());
        assert!(subqueries.iter().any(|q| q.query.contains("React")));
        assert!(subqueries.iter().any(|q| q.query.contains("Vue")));
    }

    #[test]
    fn test_decompose_simple_query_no_change() {
        let router = SearchRouter::new();
        let subqueries = router.decompose_into_subqueries("simple query");
        assert_eq!(subqueries.len(), 1);
        assert_eq!(subqueries[0].query, "simple query");
    }

    #[test]
    fn test_routing_decision_quick() {
        let router = SearchRouter::new();
        let decision = router.route("error: cannot find symbol");
        assert_eq!(decision.depth, SearchDepth::Quick);
        assert!(decision.sub_queries.is_empty());
    }

    #[test]
    fn test_routing_decision_deep() {
        let router = SearchRouter::new();
        let decision =
            router.route("How to implement a custom async runtime in Rust with work stealing");
        assert_eq!(decision.depth, SearchDepth::Deep);
        assert!(!decision.sub_queries.is_empty());
    }

    #[test]
    fn test_search_depth_serialization() {
        let depth = SearchDepth::Quick;
        let json = serde_json::to_string(&depth).unwrap();
        assert_eq!(json, "\"Quick\"");
        let parsed: SearchDepth = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, SearchDepth::Quick);
    }

    #[test]
    fn test_rewritten_query_serialization() {
        let router = SearchRouter::new();
        let rewritten = router.rewrite_query("rust 1.0 async");
        let json = serde_json::to_string(&rewritten).unwrap();
        let parsed: RewrittenQuery = serde_json::from_str(&json).unwrap();
        assert!(parsed.version_detected);
    }

    #[test]
    fn test_routing_decision_serialization() {
        let router = SearchRouter::new();
        let decision = router.route("error: undefined symbol");
        let json = serde_json::to_string(&decision).unwrap();
        let parsed: RoutingDecision = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.original_query, "error: undefined symbol");
    }

    #[test]
    fn test_subquery_serialization() {
        let subquery = SubQuery {
            query: "test query".to_string(),
            rationale: "testing".to_string(),
            priority: 1,
        };
        let json = serde_json::to_string(&subquery).unwrap();
        let parsed: SubQuery = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.query, "test query");
    }
}
