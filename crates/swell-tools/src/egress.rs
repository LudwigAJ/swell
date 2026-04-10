//! Network egress control with default-deny policy and allowlist-based filtering.
//!
//! This module provides:
//! - [`EgressFilter`] - Default-deny network filter with DNS and IP allowlisting
//! - [`EgressRule`] - Individual allowlist rules for hosts and IP ranges
//! - [`EgressDecision`] - Decision outcome for egress checks
//!
//! ## Usage
//!
//! ```rust,ignore
//! use swell_tools::egress::{EgressFilter, EgressRule, Destination};
//!
//! // Create filter with default deny
//! let filter = EgressFilter::default_deny();
//!
//! // Add allowed destinations
//! filter.add_rule(EgressRule::dns("api.anthropic.com", 443));
//! filter.add_rule(EgressRule::ip("192.168.1.0/24", 80));
//!
//! // Check if destination is allowed
//! let dest = Destination::new("api.anthropic.com", 443);
//! assert!(filter.is_allowed(&dest).is_allowed());
//!
//! // Blocked destination
//! let blocked = Destination::new("evil.com", 443);
//! assert!(!filter.is_allowed(&blocked).is_allowed());
//! ```
//!
//! ## Security Model
//!
//! - **Default-deny**: All outbound traffic is blocked unless explicitly allowlisted
//! - **DNS-based restrictions**: Control access by domain name
//! - **IP-based restrictions**: Control access by IP address or CIDR range
//! - **Port restrictions**: Optional port-level filtering per rule
//! - **Cloud metadata protection**: Blocks access to 169.254.169.254 by default

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// A destination for network egress checking
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Destination {
    /// Hostname or IP address
    host: String,
    /// Port number
    port: u16,
    /// Resolved IP addresses (cached after DNS lookup)
    resolved_ips: Option<Vec<IpAddr>>,
    /// Timestamp when IPs were resolved
    resolved_at: Option<Instant>,
}

impl Destination {
    /// Create a new destination
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
            resolved_ips: None,
            resolved_at: None,
        }
    }

    /// Create from a socket address
    pub fn from_socket_addr(addr: SocketAddr) -> Self {
        Self {
            host: addr.ip().to_string(),
            port: addr.port(),
            resolved_ips: Some(vec![addr.ip()]),
            resolved_at: Some(Instant::now()),
        }
    }

    /// Get the host
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Get the port
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Check if this destination uses a cloud metadata IP
    /// Cloud providers use 169.254.169.254 for metadata service
    pub fn is_cloud_metadata_ip(&self) -> bool {
        // 169.254.169.254 - AWS, Azure, GCP metadata
        if let Ok(ip) = self.host.parse::<IpAddr>() {
            return is_cloud_metadata_ip_addr(&ip);
        }
        false
    }

    /// Resolve the hostname to IP addresses
    /// Returns cached IPs if still valid (TTL-based caching)
    pub async fn resolve(&mut self, dns_cache_ttl: Duration) -> Option<Vec<IpAddr>> {
        // Check cache validity
        if let Some(ref ips) = self.resolved_ips {
            if let Some(resolved_at) = self.resolved_at {
                if resolved_at.elapsed() < dns_cache_ttl {
                    return Some(ips.clone());
                }
            }
        }

        // Perform DNS lookup
        let lookup = tokio::net::lookup_host(format!("{}:{}", self.host, self.port)).await;

        match lookup {
            Ok(ips) => {
                let ip_vec: Vec<IpAddr> = ips.map(|s| s.ip()).collect();
                self.resolved_ips = Some(ip_vec.clone());
                self.resolved_at = Some(Instant::now());
                Some(ip_vec)
            }
            Err(e) => {
                warn!(host = %self.host, error = %e, "DNS resolution failed");
                None
            }
        }
    }

    /// Resolve synchronously (blocking)
    /// Returns cached IPs if still valid
    pub fn resolve_blocking(&mut self, dns_cache_ttl: Duration) -> Option<Vec<IpAddr>> {
        use std::net::ToSocketAddrs;

        // Check cache validity
        if let Some(ref ips) = self.resolved_ips {
            if let Some(resolved_at) = self.resolved_at {
                if resolved_at.elapsed() < dns_cache_ttl {
                    return Some(ips.clone());
                }
            }
        }

        // Perform blocking DNS lookup
        let addr_string = format!("{}:{}", self.host, self.port);
        let mut addrs = addr_string.to_socket_addrs().ok();

        // Try to get at least one IP
        if let Some(ref mut iter) = addrs {
            let ip_vec: Vec<IpAddr> = iter.map(|s| s.ip()).collect();
            if !ip_vec.is_empty() {
                self.resolved_ips = Some(ip_vec.clone());
                self.resolved_at = Some(Instant::now());
                return Some(ip_vec);
            }
        }

        warn!(host = %self.host, "DNS resolution failed");
        None
    }

    /// Get cached IPs without triggering resolution
    pub fn cached_ips(&self) -> Option<&[IpAddr]> {
        self.resolved_ips.as_deref()
    }
}

impl std::fmt::Display for Destination {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

/// Check if an IP address is a cloud metadata address
fn is_cloud_metadata_ip_addr(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            // 169.254.169.254 - AWS, Azure, GCP, Oracle cloud metadata
            // 169.254.169.251-254 - GCP/Azure metadata/DNS servers
            let octets = v4.octets();
            octets[0] == 169 && octets[1] == 254 && octets[2] == 169 && octets[3] >= 251
        }
        IpAddr::V6(_) => {
            // IPv6 link-local for metadata (fe80::/10 range, specifically fe80::1)
            false // IPv6 metadata not commonly used yet
        }
    }
}

/// An individual egress allowlist rule
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EgressRule {
    /// Allow by DNS hostname and optional port
    Dns {
        /// The hostname to allow (e.g., "api.anthropic.com")
        hostname: String,
        /// Optional port restriction (None = all ports)
        port: Option<u16>,
    },
    /// Allow by IP address and optional port
    Ip {
        /// The IP address or CIDR range
        address: IpNetwork,
        /// Optional port restriction (None = all ports)
        port: Option<u16>,
    },
}

impl EgressRule {
    /// Create a DNS-based rule allowing all ports
    pub fn dns(hostname: impl Into<String>, port: u16) -> Self {
        Self::Dns {
            hostname: hostname.into(),
            port: Some(port),
        }
    }

    /// Create a DNS-based rule allowing all ports
    pub fn dns_all_ports(hostname: impl Into<String>) -> Self {
        Self::Dns {
            hostname: hostname.into(),
            port: None,
        }
    }

    /// Create an IP-based rule
    pub fn ip(address: impl Into<IpNetwork>, port: u16) -> Self {
        Self::Ip {
            address: address.into(),
            port: Some(port),
        }
    }

    /// Create an IP-based rule allowing all ports
    pub fn ip_all_ports(address: impl Into<IpNetwork>) -> Self {
        Self::Ip {
            address: address.into(),
            port: None,
        }
    }

    /// Check if this rule matches the given destination
    pub fn matches(&self, dest: &Destination) -> bool {
        match self {
            EgressRule::Dns { hostname, port } => {
                // Check hostname match (exact or suffix for subdomains)
                let dest_host_lower = dest.host.to_lowercase();
                let rule_host_lower = hostname.to_lowercase();

                let hostname_matches = dest_host_lower == rule_host_lower
                    || dest_host_lower.ends_with(&format!(".{}", rule_host_lower));

                if !hostname_matches {
                    return false;
                }

                // Check port if specified
                if let Some(rule_port) = port {
                    return dest.port == *rule_port;
                }

                true
            }
            EgressRule::Ip { address, port } => {
                // Try to match against resolved IPs or the host string
                let ips = dest.cached_ips();

                let ip_matches = if let Some(ips) = ips {
                    ips.iter().any(|ip| address.contains(*ip))
                } else {
                    // Try to parse host as IP and check
                    if let Ok(dest_ip) = dest.host.parse::<IpAddr>() {
                        address.contains(dest_ip)
                    } else {
                        false
                    }
                };

                if !ip_matches {
                    return false;
                }

                // Check port if specified
                if let Some(rule_port) = port {
                    return dest.port == *rule_port;
                }

                true
            }
        }
    }

    /// Get a description of this rule for logging
    pub fn description(&self) -> String {
        match self {
            EgressRule::Dns { hostname, port } => {
                if let Some(p) = port {
                    format!("dns:{}:{}", hostname, p)
                } else {
                    format!("dns:{}:*", hostname)
                }
            }
            EgressRule::Ip { address, port } => {
                if let Some(p) = port {
                    format!("ip:{}:{}", address, p)
                } else {
                    format!("ip:{}:*", address)
                }
            }
        }
    }
}

/// An IP network (address + prefix length)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IpNetwork {
    /// The IP address
    address: IpAddr,
    /// The prefix length (CIDR notation)
    prefix_len: u8,
}

impl IpNetwork {
    /// Parse a CIDR notation string (e.g., "192.168.1.0/24" or "10.0.0.1/32")
    pub fn parse_cidr(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split('/').collect::<Vec<_>>();
        if parts.len() != 2 {
            // Try parsing as just an IP address (prefix_len = 32 for IPv4, 128 for IPv6)
            let ip: IpAddr = s.parse().ok()?;
            let prefix_len = match ip {
                IpAddr::V4(_) => 32,
                IpAddr::V6(_) => 128,
            };
            return Some(Self {
                address: ip,
                prefix_len,
            });
        }

        let ip: IpAddr = parts[0].parse().ok()?;
        let prefix_len: u8 = parts[1].parse().ok()?;

        // Validate prefix length
        let max_prefix = match ip {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };

        if prefix_len > max_prefix {
            return None;
        }

        Some(Self {
            address: ip,
            prefix_len,
        })
    }

    /// Create from an IP address with full prefix (single host)
    pub fn single(ip: IpAddr) -> Self {
        let prefix_len = match ip {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        Self {
            address: ip,
            prefix_len,
        }
    }

    /// Check if this network contains the given IP address
    pub fn contains(&self, ip: IpAddr) -> bool {
        // Both must be the same version
        match (self.address, ip) {
            (IpAddr::V4(addr_v4), IpAddr::V4(ip_v4)) => self.contains_v4(addr_v4, ip_v4),
            (IpAddr::V6(addr_v6), IpAddr::V6(ip_v6)) => self.contains_v6(addr_v6, ip_v6),
            _ => false,
        }
    }

    fn contains_v4(&self, network: Ipv4Addr, ip: Ipv4Addr) -> bool {
        let network_u32 = u32::from(network);
        let ip_u32 = u32::from(ip);
        let mask = if self.prefix_len == 0 {
            0
        } else {
            !0u32 << (32 - self.prefix_len)
        };
        (network_u32 & mask) == (ip_u32 & mask)
    }

    fn contains_v6(&self, network: Ipv6Addr, ip: Ipv6Addr) -> bool {
        let network_segments = network.segments();
        let ip_segments = ip.segments();
        let prefix_len = self.prefix_len as usize;

        for i in 0..8 {
            let bits_to_check = if prefix_len > (i * 16) {
                (prefix_len - (i * 16)).min(16) as u32
            } else {
                0
            };

            if bits_to_check == 0 {
                continue;
            }

            let mask = !0u16 << (16 - bits_to_check);

            if (network_segments[i] & mask) != (ip_segments[i] & mask) {
                return false;
            }
        }
        true
    }
}

impl std::fmt::Display for IpNetwork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Only omit prefix if it's the "full" prefix (32 for IPv4, 128 for IPv6)
        let is_full_prefix = match self.address {
            IpAddr::V4(_) => self.prefix_len == 32,
            IpAddr::V6(_) => self.prefix_len == 128,
        };

        if is_full_prefix {
            write!(f, "{}", self.address)
        } else {
            write!(f, "{}/{}", self.address, self.prefix_len)
        }
    }
}

impl Serialize for IpNetwork {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for IpNetwork {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        IpNetwork::parse_cidr(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid IP network: {}", s)))
    }
}

impl From<IpAddr> for IpNetwork {
    fn from(ip: IpAddr) -> Self {
        Self::single(ip)
    }
}

impl From<&str> for IpNetwork {
    fn from(s: &str) -> Self {
        IpNetwork::parse_cidr(s).unwrap_or_else(|| {
            // Fallback: try parsing as IP
            let ip: IpAddr = s.parse().unwrap_or_else(|_| {
                // If all else fails, use 0.0.0.0 which won't match anything
                IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0))
            });
            Self::single(ip)
        })
    }
}

impl From<String> for IpNetwork {
    fn from(s: String) -> Self {
        IpNetwork::parse_cidr(&s).unwrap_or_else(|| IpNetwork::from(s.as_str()))
    }
}

/// Result of an egress check
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EgressDecision {
    /// Connection is allowed
    Allowed,
    /// Connection is denied (default-deny or explicit block)
    Denied,
}

impl EgressDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, EgressDecision::Allowed)
    }

    pub fn reason(&self) -> &'static str {
        match self {
            EgressDecision::Allowed => "allowed",
            EgressDecision::Denied => "denied by default-deny policy",
        }
    }
}

/// Detailed egress check result with reason
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EgressCheckResult {
    pub decision: EgressDecision,
    pub reason: String,
    pub matched_rule: Option<String>,
    pub destination: String,
}

impl EgressCheckResult {
    /// Create an allowed result
    fn allowed(destination: &Destination, rule: &EgressRule) -> Self {
        Self {
            decision: EgressDecision::Allowed,
            reason: format!("allowed by rule: {}", rule.description()),
            matched_rule: Some(rule.description()),
            destination: destination.to_string(),
        }
    }

    /// Create a denied result
    fn denied_default(destination: &Destination) -> Self {
        Self {
            decision: EgressDecision::Denied,
            reason: "default-deny: no allowlist rule matched".to_string(),
            matched_rule: None,
            destination: destination.to_string(),
        }
    }

    /// Convenience method to check if the destination is allowed
    pub fn is_allowed(&self) -> bool {
        self.decision.is_allowed()
    }

    /// Create a denied result due to cloud metadata
    fn denied_cloud_metadata(destination: &Destination) -> Self {
        Self {
            decision: EgressDecision::Denied,
            reason: "denied: cloud metadata IP (169.254.169.254) is always blocked".to_string(),
            matched_rule: None,
            destination: destination.to_string(),
        }
    }
}

/// DNS cache entry
#[derive(Debug, Clone)]
struct DnsCacheEntry {
    #[allow(dead_code)]
    ips: Vec<IpAddr>, // Reserved for future use with per-IP allowlists
    cached_at: Instant,
}

/// Configuration for the egress filter
#[derive(Debug, Clone)]
pub struct EgressFilterConfig {
    /// DNS cache TTL
    pub dns_cache_ttl: Duration,
    /// Block cloud metadata IPs (169.254.169.254)
    pub block_cloud_metadata: bool,
    /// Log all decisions (including allows)
    pub log_all_decisions: bool,
    /// Log blocked connections
    pub log_blocked: bool,
}

impl Default for EgressFilterConfig {
    fn default() -> Self {
        Self {
            dns_cache_ttl: Duration::from_secs(300), // 5 minutes
            block_cloud_metadata: true,
            log_all_decisions: false,
            log_blocked: true,
        }
    }
}

/// Default-deny network egress filter with allowlist-based permitting
#[derive(Debug, Clone)]
pub struct EgressFilter {
    /// Rules that explicitly allow connections
    rules: Vec<EgressRule>,
    /// Configuration
    config: EgressFilterConfig,
    /// DNS cache for resolved hostnames
    dns_cache: Arc<RwLock<HashMap<String, DnsCacheEntry>>>,
}

impl Default for EgressFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl EgressFilter {
    /// Create a new egress filter with default configuration
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            config: EgressFilterConfig::default(),
            dns_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create an egress filter with default-deny policy
    /// This is the recommended constructor - all traffic is blocked unless allowlisted
    pub fn default_deny() -> Self {
        Self::new()
    }

    /// Create an egress filter with a custom configuration
    pub fn with_config(config: EgressFilterConfig) -> Self {
        Self {
            rules: Vec::new(),
            config,
            dns_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add an allowlist rule
    pub fn add_rule(&mut self, rule: EgressRule) {
        info!(rule = %rule.description(), "Adding egress allowlist rule");
        self.rules.push(rule);
    }

    /// Add multiple rules at once
    pub fn add_rules(&mut self, rules: impl IntoIterator<Item = EgressRule>) {
        for rule in rules {
            self.add_rule(rule);
        }
    }

    /// Get a list of all rules
    pub fn rules(&self) -> &[EgressRule] {
        &self.rules
    }

    /// Get the configuration
    pub fn config(&self) -> &EgressFilterConfig {
        &self.config
    }

    /// Clear all rules
    pub fn clear_rules(&mut self) {
        self.rules.clear();
    }

    /// Remove a specific rule (by description)
    pub fn remove_rule(&mut self, rule_description: &str) -> bool {
        let initial_len = self.rules.len();
        self.rules.retain(|r| r.description() != rule_description);
        self.rules.len() < initial_len
    }

    /// Check if a destination is allowed
    /// This performs DNS resolution if needed and checks against allowlist rules
    pub async fn is_allowed(&self, destination: &Destination) -> EgressCheckResult {
        self.check_with_resolution(destination).await
    }

    /// Check if a destination is allowed (synchronous version)
    /// Uses cached DNS lookups only
    pub fn is_allowed_sync(&self, destination: &Destination) -> EgressCheckResult {
        self.check_sync(destination)
    }

    /// Check with DNS resolution
    async fn check_with_resolution(&self, destination: &Destination) -> EgressCheckResult {
        // First, check for cloud metadata IP blocks
        if self.config.block_cloud_metadata && destination.is_cloud_metadata_ip() {
            debug!(dest = %destination, "Egress denied: cloud metadata IP");
            return EgressCheckResult::denied_cloud_metadata(destination);
        }

        // Check against rules - first do cached IP check
        let result = self.check_sync(destination);

        if result.decision == EgressDecision::Allowed {
            return result;
        }

        // If cached check didn't match and we have a hostname, try resolving
        if destination.host.contains('.') && destination.host.parse::<IpAddr>().is_err() {
            let mut dest_clone = destination.clone();
            if let Some(ips) = dest_clone.resolve(self.config.dns_cache_ttl).await {
                // Try matching against resolved IPs
                for ip in &ips {
                    if self.config.block_cloud_metadata && is_cloud_metadata_ip_addr(ip) {
                        debug!(dest = %destination, ip = %ip, "Egress denied: cloud metadata IP after resolution");
                        return EgressCheckResult::denied_cloud_metadata(destination);
                    }
                }
            }
        }

        // Re-check after potential resolution
        self.check_sync(destination)
    }

    /// Synchronous check (uses cached IPs only)
    fn check_sync(&self, destination: &Destination) -> EgressCheckResult {
        // CRITICAL: Check cloud metadata IPs first, regardless of rules
        // Cloud metadata should NEVER be accessible, even if a user explicitly allows it
        if self.config.block_cloud_metadata && destination.is_cloud_metadata_ip() {
            debug!(dest = %destination, "Egress denied: cloud metadata IP (blocked regardless of rules)");
            return EgressCheckResult::denied_cloud_metadata(destination);
        }

        // Check against rules
        for rule in &self.rules {
            if rule.matches(destination) {
                if self.config.log_all_decisions {
                    debug!(dest = %destination, rule = %rule.description(), "Egress allowed");
                }
                return EgressCheckResult::allowed(destination, rule);
            }
        }

        // No rule matched - default deny
        if self.config.log_blocked {
            warn!(dest = %destination, "Egress denied: no matching allowlist rule");
        }
        EgressCheckResult::denied_default(destination)
    }

    /// Create a check future for a destination
    pub fn check(&self, destination: Destination) -> EgressCheckFuture {
        EgressCheckFuture {
            filter: self.clone(),
            destination,
        }
    }

    /// Clear the DNS cache
    pub async fn clear_dns_cache(&self) {
        let mut cache = self.dns_cache.write().await;
        cache.clear();
    }

    /// Prune expired DNS cache entries
    pub async fn prune_dns_cache(&self) {
        let mut cache = self.dns_cache.write().await;
        cache.retain(|_, entry| entry.cached_at.elapsed() < self.config.dns_cache_ttl);
    }
}

/// Future for async egress checking
#[derive(Debug, Clone)]
pub struct EgressCheckFuture {
    filter: EgressFilter,
    destination: Destination,
}

impl std::future::Future for EgressCheckFuture {
    type Output = EgressCheckResult;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        // Use synchronous check since DNS resolution is cached
        // The async resolution happens on first call, subsequent calls use cache
        std::task::Poll::Ready(self.filter.check_sync(&self.destination))
    }
}

/// Predefined rule sets for common use cases
pub mod presets {
    use super::*;

    /// Rules for LLM API access only
    pub fn llm_api_only() -> Vec<EgressRule> {
        vec![
            // Anthropic
            EgressRule::dns_all_ports("api.anthropic.com"),
            EgressRule::dns_all_ports("auth.anthropic.com"),
            // OpenAI
            EgressRule::dns_all_ports("api.openai.com"),
            EgressRule::dns_all_ports("api.chatanywhere.tech"),
            // Google AI
            EgressRule::dns_all_ports("generativelanguage.googleapis.com"),
            // Azure OpenAI
            EgressRule::dns_all_ports("*.openai.azure.com"),
            // Cloudflare AI
            EgressRule::dns_all_ports("api.cloudflare.com"),
        ]
    }

    /// Rules for package registry access
    pub fn package_registries() -> Vec<EgressRule> {
        vec![
            // crates.io
            EgressRule::dns_all_ports("crates.io"),
            // npm
            EgressRule::dns_all_ports("registry.npmjs.org"),
            EgressRule::dns_all_ports("registry.npmmirror.com"),
            // PyPI
            EgressRule::dns_all_ports("pypi.org"),
            EgressRule::dns_all_ports("pypi.python.org"),
            // Maven
            EgressRule::dns_all_ports("repo.maven.apache.org"),
            // Go
            EgressRule::dns_all_ports("proxy.golang.org"),
            EgressRule::dns_all_ports("sum.golang.org"),
        ]
    }

    /// Rules for git operations
    pub fn git_operations() -> Vec<EgressRule> {
        vec![
            // GitHub
            EgressRule::dns_all_ports("github.com"),
            EgressRule::dns_all_ports("githubusercontent.com"),
            // GitLab
            EgressRule::dns_all_ports("gitlab.com"),
            // Bitbucket
            EgressRule::dns_all_ports("bitbucket.org"),
        ]
    }

    /// Minimal rules for local-only operation (no network)
    pub fn local_only() -> Vec<EgressRule> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_destination_basic() {
        let dest = Destination::new("api.anthropic.com", 443);
        assert_eq!(dest.host(), "api.anthropic.com");
        assert_eq!(dest.port(), 443);
    }

    #[test]
    fn test_destination_cloud_metadata_ip() {
        // AWS metadata IP
        let dest = Destination::new("169.254.169.254", 80);
        assert!(dest.is_cloud_metadata_ip());

        // Azure metadata IP
        let dest2 = Destination::new("169.254.169.253", 80);
        assert!(dest2.is_cloud_metadata_ip());

        // GCP metadata IPs
        let dest3 = Destination::new("169.254.169.251", 80);
        assert!(dest3.is_cloud_metadata_ip());

        // Non-metadata IP
        let dest4 = Destination::new("1.2.3.4", 80);
        assert!(!dest4.is_cloud_metadata_ip());

        // Regular hostname
        let dest5 = Destination::new("api.anthropic.com", 443);
        assert!(!dest5.is_cloud_metadata_ip());
    }

    #[test]
    fn test_ip_network_parse_cidr() {
        let net = IpNetwork::parse_cidr("192.168.1.0/24").unwrap();
        assert_eq!(net.to_string(), "192.168.1.0/24");

        // Single IP
        let net = IpNetwork::parse_cidr("192.168.1.1/32").unwrap();
        assert_eq!(net.to_string(), "192.168.1.1");

        // Just IP (no CIDR)
        let net = IpNetwork::parse_cidr("10.0.0.1").unwrap();
        assert_eq!(net.to_string(), "10.0.0.1");

        // IPv6
        let net = IpNetwork::parse_cidr("2001:db8::/32").unwrap();
        assert_eq!(net.to_string(), "2001:db8::/32");
    }

    #[test]
    fn test_ip_network_contains() {
        // /24 network
        let net = IpNetwork::parse_cidr("192.168.1.0/24").unwrap();
        assert!(net.contains("192.168.1.0".parse().unwrap()));
        assert!(net.contains("192.168.1.255".parse().unwrap()));
        assert!(net.contains("192.168.1.100".parse().unwrap()));
        assert!(!net.contains("192.168.2.1".parse().unwrap()));

        // /32 (single host)
        let net = IpNetwork::parse_cidr("10.0.0.1/32").unwrap();
        assert!(net.contains("10.0.0.1".parse().unwrap()));
        assert!(!net.contains("10.0.0.2".parse().unwrap()));
    }

    #[test]
    fn test_egress_rule_dns_match() {
        let rule = EgressRule::dns("api.anthropic.com", 443);
        let dest = Destination::new("api.anthropic.com", 443);
        assert!(rule.matches(&dest));

        // Port mismatch
        let dest_wrong_port = Destination::new("api.anthropic.com", 80);
        assert!(!rule.matches(&dest_wrong_port));

        // Hostname mismatch
        let dest_wrong_host = Destination::new("api.openai.com", 443);
        assert!(!rule.matches(&dest_wrong_host));
    }

    #[test]
    fn test_egress_rule_dns_subdomain_match() {
        let rule = EgressRule::dns_all_ports("anthropic.com");
        let dest = Destination::new("api.anthropic.com", 443);
        assert!(rule.matches(&dest));

        let dest2 = Destination::new("www.anthropic.com", 80);
        assert!(rule.matches(&dest2));

        let dest3 = Destination::new("api.openai.com", 443);
        assert!(!rule.matches(&dest3));
    }

    #[test]
    fn test_egress_rule_ip_match() {
        let net = IpNetwork::parse_cidr("192.168.1.0/24").unwrap();
        let rule = EgressRule::ip(net, 80);

        let dest = Destination::new("192.168.1.100", 80);
        assert!(rule.matches(&dest));

        // Wrong port
        let dest_wrong_port = Destination::new("192.168.1.100", 443);
        assert!(!rule.matches(&dest_wrong_port));

        // Out of range
        let dest_out_of_range = Destination::new("192.168.2.1", 80);
        assert!(!rule.matches(&dest_out_of_range));
    }

    #[test]
    fn test_egress_filter_default_deny() {
        let filter = EgressFilter::default_deny();

        let dest = Destination::new("api.anthropic.com", 443);
        let result = filter.is_allowed_sync(&dest);
        assert!(!result.is_allowed());
    }

    #[test]
    fn test_egress_filter_allowlist() {
        let mut filter = EgressFilter::default_deny();
        filter.add_rule(EgressRule::dns("api.anthropic.com", 443));

        let allowed = Destination::new("api.anthropic.com", 443);
        assert!(filter.is_allowed_sync(&allowed).is_allowed());

        let blocked = Destination::new("evil.com", 443);
        assert!(!filter.is_allowed_sync(&blocked).is_allowed());
    }

    #[test]
    fn test_egress_filter_cloud_metadata_blocked() {
        let mut filter = EgressFilter::default_deny();
        filter.add_rule(EgressRule::ip("0.0.0.0/0", 80)); // Allow all on port 80

        // Cloud metadata should still be blocked
        let metadata_dest = Destination::new("169.254.169.254", 80);
        let result = filter.is_allowed_sync(&metadata_dest);
        assert!(!result.is_allowed());
    }

    #[test]
    fn test_egress_filter_multiple_rules() {
        let mut filter = EgressFilter::default_deny();
        filter.add_rules([
            EgressRule::dns("api.anthropic.com", 443),
            EgressRule::ip("192.168.1.0/24", 80),
        ]);

        // DNS rule
        assert!(filter
            .is_allowed_sync(&Destination::new("api.anthropic.com", 443))
            .is_allowed());

        // IP rule
        assert!(filter
            .is_allowed_sync(&Destination::new("192.168.1.50", 80))
            .is_allowed());

        // Blocked
        assert!(!filter
            .is_allowed_sync(&Destination::new("evil.com", 443))
            .is_allowed());
    }

    #[test]
    fn test_presets_llm_api_only() {
        let rules = presets::llm_api_only();
        assert!(!rules.is_empty());

        let mut filter = EgressFilter::default_deny();
        filter.add_rules(rules);

        // Anthropic should be allowed
        assert!(filter
            .is_allowed_sync(&Destination::new("api.anthropic.com", 443))
            .is_allowed());

        // Random site should be blocked
        assert!(!filter
            .is_allowed_sync(&Destination::new("evil.com", 443))
            .is_allowed());
    }

    #[test]
    fn test_presets_package_registries() {
        let rules = presets::package_registries();
        assert!(!rules.is_empty());

        let mut filter = EgressFilter::default_deny();
        filter.add_rules(rules);

        // crates.io should be allowed
        assert!(filter
            .is_allowed_sync(&Destination::new("crates.io", 443))
            .is_allowed());

        // pypi should be allowed
        assert!(filter
            .is_allowed_sync(&Destination::new("pypi.org", 443))
            .is_allowed());
    }

    #[tokio::test]
    async fn test_egress_filter_clear_rules() {
        let mut filter = EgressFilter::default_deny();
        filter.add_rule(EgressRule::dns("api.anthropic.com", 443));

        assert!(filter
            .is_allowed_sync(&Destination::new("api.anthropic.com", 443))
            .is_allowed());

        filter.clear_rules();

        assert!(!filter
            .is_allowed_sync(&Destination::new("api.anthropic.com", 443))
            .is_allowed());
    }

    #[tokio::test]
    async fn test_egress_filter_remove_rule() {
        let mut filter = EgressFilter::default_deny();
        filter.add_rule(EgressRule::dns("api.anthropic.com", 443));

        assert!(filter
            .is_allowed_sync(&Destination::new("api.anthropic.com", 443))
            .is_allowed());

        let removed = filter.remove_rule("dns:api.anthropic.com:443");
        assert!(removed);

        assert!(!filter
            .is_allowed_sync(&Destination::new("api.anthropic.com", 443))
            .is_allowed());

        // Removing non-existent rule should return false
        let removed_again = filter.remove_rule("dns:api.anthropic.com:443");
        assert!(!removed_again);
    }

    #[test]
    fn test_destination_from_socket_addr() {
        let addr: SocketAddr = "192.168.1.1:8080".parse().unwrap();
        let dest = Destination::from_socket_addr(addr);

        assert_eq!(dest.host(), "192.168.1.1");
        assert_eq!(dest.port(), 8080);
        assert!(dest.cached_ips().is_some());
    }

    #[test]
    fn test_egress_decision() {
        let allowed = EgressDecision::Allowed;
        assert!(allowed.is_allowed());
        assert_eq!(allowed.reason(), "allowed");

        let denied = EgressDecision::Denied;
        assert!(!denied.is_allowed());
    }

    #[test]
    fn test_egress_filter_clone() {
        let mut filter = EgressFilter::default_deny();
        filter.add_rule(EgressRule::dns("api.anthropic.com", 443));

        let cloned = filter.clone();
        assert!(cloned
            .is_allowed_sync(&Destination::new("api.anthropic.com", 443))
            .is_allowed());
    }

    #[test]
    fn test_dns_all_ports_rule() {
        let rule = EgressRule::dns_all_ports("api.example.com");
        let dest80 = Destination::new("api.example.com", 80);
        let dest443 = Destination::new("api.example.com", 443);
        let dest8080 = Destination::new("api.example.com", 8080);

        assert!(rule.matches(&dest80));
        assert!(rule.matches(&dest443));
        assert!(rule.matches(&dest8080));
    }

    #[test]
    fn test_ip_all_ports_rule() {
        let net = IpNetwork::parse_cidr("10.0.0.0/8").unwrap();
        let rule = EgressRule::ip_all_ports(net);
        let dest80 = Destination::new("10.50.100.200", 80);
        let dest443 = Destination::new("10.50.100.200", 443);

        assert!(rule.matches(&dest80));
        assert!(rule.matches(&dest443));
    }
}
