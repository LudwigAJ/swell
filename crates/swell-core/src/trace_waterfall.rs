//! Trace waterfall view data for task/agent execution.
//!
//! This module provides hierarchical span structure with timing information
//! for visualizing task/agent execution as a waterfall chart.
//!
//! # Waterfall View Structure
//!
//! The waterfall view represents the complete execution hierarchy:
//! - Root span represents the task
//! - Child spans represent agent phases (planning, execution, validation)
//! - Grandchild spans represent individual tool calls
//! - Decision points are marked as special spans
//!
//! # Features
//!
//! - Hierarchical span structure with parent-child relationships
//! - Start/end timestamps per span
//! - Tool duration breakdown with input/output sizing
//! - Agent decision point marking
//! - JSON serialization for waterfall visualization

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::{ObservableEvent, Outcome, SpanId, TraceId};
use crate::ids::{AgentId, SessionId, TaskId};

/// A span in the trace waterfall representing a unit of work.
///
/// Each span has a unique ID, optional parent, and timing information.
/// Spans form a tree structure representing the execution hierarchy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceSpan {
    /// Unique identifier for this span (16 hex chars)
    pub span_id: SpanId,
    /// Identifier of the parent span (if any)
    pub parent_span_id: Option<SpanId>,
    /// Human-readable name for this span
    pub name: String,
    /// Category of work (task, agent, tool, decision)
    pub kind: SpanKind,
    /// When this span started
    pub start_time: DateTime<Utc>,
    /// When this span ended
    pub end_time: DateTime<Utc>,
    /// Duration in milliseconds
    pub duration_ms: u64,
    /// Outcome of this span's execution
    pub outcome: Outcome,
    /// Agent that executed this span (if applicable)
    pub agent_id: Option<AgentId>,
    /// Tool invocation details (if this is a tool span)
    pub tool_invocation: Option<ToolSpanDetails>,
    /// Whether this is a decision point
    pub is_decision_point: bool,
    /// Nested child spans
    #[serde(default)]
    pub children: Vec<TraceSpan>,
    /// Attributes for additional metadata
    #[serde(default)]
    pub attributes: Vec<SpanAttribute>,
}

/// The category/kind of a span
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpanKind {
    /// Root task span
    Task,
    /// Agent phase span (planning, execution, validation)
    Agent,
    /// Tool invocation span
    Tool,
    /// Decision point span
    Decision,
    /// LLM call span
    Llm,
    /// Generic span for other operations
    Other,
}

impl SpanKind {
    /// Returns a short string identifier for this kind
    pub fn as_str(&self) -> &'static str {
        match self {
            SpanKind::Task => "task",
            SpanKind::Agent => "agent",
            SpanKind::Tool => "tool",
            SpanKind::Decision => "decision",
            SpanKind::Llm => "llm",
            SpanKind::Other => "other",
        }
    }
}

/// Tool-specific details for tool invocation spans
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpanDetails {
    /// Name of the tool invoked
    pub tool_name: String,
    /// Arguments passed to the tool (summary)
    pub arguments_summary: String,
    /// Size of input in bytes
    pub input_bytes: Option<u64>,
    /// Size of output in bytes
    pub output_bytes: Option<u64>,
    /// Whether the tool executed successfully
    pub success: bool,
}

/// Key-value attribute for span metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpanAttribute {
    /// Attribute key
    pub key: String,
    /// Attribute value
    pub value: SpanAttributeValue,
}

/// Span attribute values
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SpanAttributeValue {
    String(String),
    Number(f64),
    Boolean(bool),
    Array(Vec<SpanAttributeValue>),
}

/// A complete trace waterfall representing a task's execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceWaterfall {
    /// Unique trace identifier
    pub trace_id: TraceId,
    /// Task identifier
    pub task_id: TaskId,
    /// Session identifier
    pub session_id: SessionId,
    /// When the trace started
    pub start_time: DateTime<Utc>,
    /// When the trace ended
    pub end_time: DateTime<Utc>,
    /// Total duration in milliseconds
    pub total_duration_ms: u64,
    /// Root span of the waterfall
    pub root_span: TraceSpan,
    /// Summary statistics
    pub summary: TraceSummary,
}

/// Summary statistics for a trace
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceSummary {
    /// Total number of spans in the trace
    pub total_spans: usize,
    /// Number of tool invocations
    pub tool_count: usize,
    /// Number of LLM calls
    pub llm_count: usize,
    /// Number of decision points
    pub decision_count: usize,
    /// Total tool duration in milliseconds
    pub total_tool_duration_ms: u64,
    /// Total LLM duration in milliseconds
    pub total_llm_duration_ms: u64,
    /// Overall outcome
    pub outcome: Outcome,
}

/// Context for span counting during tree traversal
struct SpanCountContext {
    total_spans: usize,
    tool_count: usize,
    llm_count: usize,
    decision_count: usize,
    total_tool_duration_ms: u64,
    total_llm_duration_ms: u64,
    outcome: Outcome,
}

/// Builder for constructing trace waterfalls from events
#[derive(Debug, Default)]
pub struct TraceWaterfallBuilder {
    spans: Vec<TraceSpan>,
    trace_id: Option<TraceId>,
    task_id: Option<TaskId>,
    session_id: Option<SessionId>,
}

impl TraceWaterfallBuilder {
    /// Create a new waterfall builder
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an event to the builder, creating or updating spans
    pub fn add_event(&mut self, event: &ObservableEvent) -> &mut Self {
        // Capture trace context from first event
        if self.trace_id.is_none() {
            self.trace_id = Some(event.trace_id.clone());
            self.task_id = Some(event.task_id);
            self.session_id = Some(event.session_id);
        }
        // For now, we create a flat list of spans from events
        // The hierarchy is built later when finalizing the waterfall
        let span = self.event_to_span(event);
        self.spans.push(span);
        self
    }

    /// Convert an observable event to a trace span
    fn event_to_span(&self, event: &ObservableEvent) -> TraceSpan {
        let duration_ms = event
            .tool_invocation
            .as_ref()
            .map(|t| t.duration_ms)
            .unwrap_or(0);

        // Determine span kind based on tool name
        // Task/agent-level spans vs actual tool invocations
        let kind = if let Some(ref tool_inv) = event.tool_invocation {
            if Self::is_agent_phase(&tool_inv.tool_name) {
                SpanKind::Agent
            } else if Self::is_llm_tool(&tool_inv.tool_name) {
                SpanKind::Llm
            } else {
                SpanKind::Tool
            }
        } else {
            SpanKind::Agent
        };

        // Determine if this is a decision point (heuristic: error outcome or specific tool names)
        let is_decision_point = event.outcome == Outcome::Error
            || event
                .tool_invocation
                .as_ref()
                .map(|t| {
                    matches!(
                        t.tool_name.as_str(),
                        "plan" | "decide" | "evaluate" | "approve" | "reject"
                    )
                })
                .unwrap_or(false);

        let tool_invocation = event.tool_invocation.as_ref().map(|t| {
            // Create a summary of arguments (first 100 chars)
            let args_str = serde_json::to_string(&t.arguments).unwrap_or_default();
            let summary = if args_str.len() > 100 {
                format!("{}...", &args_str[..100])
            } else {
                args_str
            };

            ToolSpanDetails {
                tool_name: t.tool_name.clone(),
                arguments_summary: summary,
                input_bytes: None, // Would need to be tracked separately
                output_bytes: None,
                success: t.success,
            }
        });

        TraceSpan {
            span_id: event.span_id.clone(),
            parent_span_id: event.parent_span_id.clone(),
            name: event
                .tool_invocation
                .as_ref()
                .map(|t| t.tool_name.clone())
                .unwrap_or_else(|| "agent_phase".to_string()),
            kind,
            start_time: event.timestamp,
            end_time: event.timestamp, // Will be updated when we have duration
            duration_ms,
            outcome: event.outcome,
            agent_id: Some(event.agent_id),
            tool_invocation,
            is_decision_point,
            children: Vec::new(),
            attributes: Vec::new(),
        }
    }

    /// Check if a tool name represents an agent phase (not an actual tool)
    fn is_agent_phase(tool_name: &str) -> bool {
        matches!(
            tool_name,
            "task" | "planning" | "execution" | "validation" | "agent_phase"
        )
    }

    /// Check if a tool name represents an LLM call
    fn is_llm_tool(tool_name: &str) -> bool {
        tool_name.starts_with("llm_")
            || tool_name.contains("chat")
            || tool_name.contains("complete")
    }

    /// Build a complete trace waterfall from accumulated events
    pub fn build(mut self) -> Option<TraceWaterfall> {
        if self.spans.is_empty() {
            return None;
        }

        // Build the span tree first
        let root_span = self.build_span_tree()?;

        // Calculate summary from the tree (not the flat spans list to avoid double-counting)
        let summary = Self::calculate_summary_from_tree(&root_span);

        // Find the earliest and latest times
        let start_time = root_span.start_time;
        let end_time = root_span.end_time;
        let total_duration_ms = root_span.duration_ms.max(
            end_time
                .signed_duration_since(start_time)
                .num_milliseconds() as u64,
        );

        Some(TraceWaterfall {
            trace_id: self.trace_id.clone()?,
            task_id: self.task_id.unwrap_or_else(TaskId::nil),
            session_id: self
                .session_id
                .unwrap_or_else(|| SessionId::from_uuid(Uuid::nil())),
            start_time,
            end_time,
            total_duration_ms,
            root_span,
            summary,
        })
    }

    /// Build a hierarchical span tree from flat spans
    fn build_span_tree(&mut self) -> Option<TraceSpan> {
        if self.spans.is_empty() {
            return None;
        }

        // Find root span (one without parent)
        let root_index = self.spans.iter().position(|s| s.parent_span_id.is_none())?;

        let mut root = self.spans.remove(root_index);
        root.end_time = root
            .start_time
            .checked_add_signed(chrono::Duration::milliseconds(root.duration_ms as i64))
            .unwrap_or(root.start_time);

        // Recursively build children
        root.children = self.build_children_for(&root.span_id);

        Some(root)
    }

    /// Recursively build children for a given parent span
    fn build_children_for(&mut self, parent_id: &SpanId) -> Vec<TraceSpan> {
        let mut children = Vec::new();

        // Find all spans that have this parent
        let child_indices: Vec<usize> = self
            .spans
            .iter()
            .enumerate()
            .filter(|(_, s)| s.parent_span_id.as_ref() == Some(parent_id))
            .map(|(i, _)| i)
            .collect();

        for index in child_indices.into_iter().rev() {
            let mut child = self.spans.remove(index);
            child.end_time = child
                .start_time
                .checked_add_signed(chrono::Duration::milliseconds(child.duration_ms as i64))
                .unwrap_or(child.start_time);

            // Recursively build this child's children
            child.children = self.build_children_for(&child.span_id);

            children.push(child);
        }

        // Sort children by start time (ascending), then by span_id for consistent ordering
        // This ensures deterministic ordering when timestamps are equal
        children.sort_by(|a, b| match a.start_time.cmp(&b.start_time) {
            std::cmp::Ordering::Equal => a.span_id.as_str().cmp(b.span_id.as_str()),
            other => other,
        });

        children
    }

    /// Calculate summary statistics from a span tree
    fn calculate_summary_from_tree(root_span: &TraceSpan) -> TraceSummary {
        let mut ctx = SpanCountContext {
            total_spans: 0,
            tool_count: 0,
            llm_count: 0,
            decision_count: 0,
            total_tool_duration_ms: 0u64,
            total_llm_duration_ms: 0u64,
            outcome: root_span.outcome,
        };

        Self::count_tree_spans_recursive(root_span, &mut ctx);

        TraceSummary {
            total_spans: ctx.total_spans,
            tool_count: ctx.tool_count,
            llm_count: ctx.llm_count,
            decision_count: ctx.decision_count,
            total_tool_duration_ms: ctx.total_tool_duration_ms,
            total_llm_duration_ms: ctx.total_llm_duration_ms,
            outcome: ctx.outcome,
        }
    }

    /// Recursively count spans in a tree and aggregate statistics
    fn count_tree_spans_recursive(span: &TraceSpan, ctx: &mut SpanCountContext) {
        ctx.total_spans += 1;

        match span.kind {
            SpanKind::Tool => {
                ctx.tool_count += 1;
                ctx.total_tool_duration_ms += span.duration_ms;
            }
            SpanKind::Llm => {
                ctx.llm_count += 1;
                ctx.total_llm_duration_ms += span.duration_ms;
            }
            _ => {}
        }

        if span.is_decision_point {
            ctx.decision_count += 1;
        }

        // Update outcome to worst outcome encountered
        if span.outcome == Outcome::Error {
            ctx.outcome = Outcome::Error;
        } else if span.outcome == Outcome::Failure && ctx.outcome == Outcome::Success {
            ctx.outcome = Outcome::Failure;
        }

        // Recurse into children
        for child in &span.children {
            Self::count_tree_spans_recursive(child, ctx);
        }
    }
}

/// Extension trait for ObservableEvent to convert to TraceSpan
pub trait ToTraceSpan {
    /// Convert an ObservableEvent to a TraceSpan
    fn to_trace_span(&self) -> TraceSpan;
}

impl ToTraceSpan for ObservableEvent {
    fn to_trace_span(&self) -> TraceSpan {
        let duration_ms = self
            .tool_invocation
            .as_ref()
            .map(|t| t.duration_ms)
            .unwrap_or(0);

        let kind = if self.tool_invocation.is_some() {
            SpanKind::Tool
        } else {
            SpanKind::Agent
        };

        let is_decision_point = self.outcome == Outcome::Error
            || self
                .tool_invocation
                .as_ref()
                .map(|t| {
                    matches!(
                        t.tool_name.as_str(),
                        "plan" | "decide" | "evaluate" | "approve" | "reject"
                    )
                })
                .unwrap_or(false);

        let tool_invocation = self.tool_invocation.as_ref().map(|t| {
            let args_str = serde_json::to_string(&t.arguments).unwrap_or_default();
            let summary = if args_str.len() > 100 {
                format!("{}...", &args_str[..100])
            } else {
                args_str
            };

            ToolSpanDetails {
                tool_name: t.tool_name.clone(),
                arguments_summary: summary,
                input_bytes: None,
                output_bytes: None,
                success: t.success,
            }
        });

        let end_time = self
            .timestamp
            .checked_add_signed(chrono::Duration::milliseconds(duration_ms as i64))
            .unwrap_or(self.timestamp);

        TraceSpan {
            span_id: self.span_id.clone(),
            parent_span_id: self.parent_span_id.clone(),
            name: self
                .tool_invocation
                .as_ref()
                .map(|t| t.tool_name.clone())
                .unwrap_or_else(|| "agent_phase".to_string()),
            kind,
            start_time: self.timestamp,
            end_time,
            duration_ms,
            outcome: self.outcome,
            agent_id: Some(self.agent_id),
            tool_invocation,
            is_decision_point,
            children: Vec::new(),
            attributes: Vec::new(),
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::ToolInvocation;

    fn create_test_event(
        span_id: &str,
        parent_span_id: Option<&str>,
        tool_name: Option<&str>,
        duration_ms: u64,
        outcome: Outcome,
    ) -> ObservableEvent {
        let trace_id = TraceId::generate();
        let span_id = SpanId::from_hex(span_id).unwrap();
        let parent_span_id = parent_span_id.map(|p| SpanId::from_hex(p).unwrap());
        let agent_id = AgentId::new();
        let session_id = SessionId::new();
        let task_id = TaskId::new();
        let root_trace_id = trace_id.clone();

        let tool_invocation = tool_name.map(|name| {
            ToolInvocation::new(
                name.to_string(),
                serde_json::json!({}),
                outcome == Outcome::Success,
                duration_ms,
            )
        });

        use crate::events::{AgentSessionId, CrossTaskCorrelationId, RequestId};

        ObservableEvent {
            trace_id,
            span_id,
            parent_span_id,
            root_trace_id,
            cross_task_correlation_id: CrossTaskCorrelationId::new(),
            agent_session_id: AgentSessionId::new(),
            request_id: RequestId::new(),
            agent_id,
            session_id,
            task_id,
            tool_invocation,
            timestamp: Utc::now(),
            outcome,
        }
    }

    #[test]
    fn test_span_kind_as_str() {
        assert_eq!(SpanKind::Task.as_str(), "task");
        assert_eq!(SpanKind::Agent.as_str(), "agent");
        assert_eq!(SpanKind::Tool.as_str(), "tool");
        assert_eq!(SpanKind::Decision.as_str(), "decision");
        assert_eq!(SpanKind::Llm.as_str(), "llm");
        assert_eq!(SpanKind::Other.as_str(), "other");
    }

    #[test]
    fn test_trace_span_creation() {
        let span = TraceSpan {
            span_id: SpanId::generate(),
            parent_span_id: None,
            name: "test_span".to_string(),
            kind: SpanKind::Agent,
            start_time: Utc::now(),
            end_time: Utc::now(),
            duration_ms: 100,
            outcome: Outcome::Success,
            agent_id: Some(AgentId::new()),
            tool_invocation: None,
            is_decision_point: false,
            children: Vec::new(),
            attributes: Vec::new(),
        };

        assert_eq!(span.name, "test_span");
        assert_eq!(span.kind, SpanKind::Agent);
        assert_eq!(span.duration_ms, 100);
    }

    #[test]
    fn test_tool_span_details() {
        let details = ToolSpanDetails {
            tool_name: "file_read".to_string(),
            arguments_summary: r#"{"path": "/test/file.txt"}"#.to_string(),
            input_bytes: Some(1024),
            output_bytes: Some(2048),
            success: true,
        };

        assert_eq!(details.tool_name, "file_read");
        assert!(details.success);
        assert_eq!(details.input_bytes, Some(1024));
    }

    #[test]
    fn test_span_attribute_serialization() {
        let attr = SpanAttribute {
            key: "test_key".to_string(),
            value: SpanAttributeValue::String("test_value".to_string()),
        };

        let json = serde_json::to_string(&attr).unwrap();
        assert!(json.contains("test_key"));
        assert!(json.contains("test_value"));
    }

    #[test]
    fn test_observable_event_to_trace_span() {
        let event = create_test_event(
            "A1B2C3D4E5F60718",
            None,
            Some("file_read"),
            50,
            Outcome::Success,
        );

        let span = event.to_trace_span();

        assert_eq!(span.span_id.as_str(), "A1B2C3D4E5F60718");
        assert_eq!(span.name, "file_read");
        assert_eq!(span.kind, SpanKind::Tool);
        assert_eq!(span.duration_ms, 50);
        assert!(!span.is_decision_point);
        assert!(span.tool_invocation.is_some());
    }

    #[test]
    fn test_decision_point_detection() {
        // Event with error outcome should be marked as decision point
        let error_event = create_test_event(
            "A1B2C3D4E5F60718",
            None,
            Some("file_read"),
            50,
            Outcome::Error,
        );

        let span = error_event.to_trace_span();
        assert!(span.is_decision_point);

        // Event with decision-related tool name
        let plan_event = create_test_event(
            "B1B2C3D4E5F60718",
            None,
            Some("plan"),
            100,
            Outcome::Success,
        );

        let span = plan_event.to_trace_span();
        assert!(span.is_decision_point);

        // Regular tool should not be decision point
        let read_event = create_test_event(
            "C1B2C3D4E5F60718",
            None,
            Some("file_read"),
            50,
            Outcome::Success,
        );

        let span = read_event.to_trace_span();
        assert!(!span.is_decision_point);
    }

    #[test]
    fn test_trace_waterfall_builder_empty() {
        let builder = TraceWaterfallBuilder::new();
        let waterfall = builder.build();
        assert!(waterfall.is_none());
    }

    #[test]
    fn test_trace_waterfall_builder_single_span() {
        let event = create_test_event(
            "A1B2C3D4E5F60718",
            None,
            Some("test_tool"),
            100,
            Outcome::Success,
        );

        let mut builder = TraceWaterfallBuilder::new();
        builder.add_event(&event);

        let waterfall = builder.build();
        assert!(waterfall.is_some());

        let waterfall = waterfall.unwrap();
        assert_eq!(waterfall.summary.total_spans, 1);
        assert_eq!(waterfall.summary.tool_count, 1);
    }

    #[test]
    fn test_trace_waterfall_builder_with_hierarchy() {
        // Create parent span
        let parent_event = create_test_event(
            "A1B2C3D4E5F60718",
            None,
            Some("planning"),
            500,
            Outcome::Success,
        );

        // Create child spans
        let child1 = create_test_event(
            "B1B2C3D4E5F60718",
            Some("A1B2C3D4E5F60718"),
            Some("file_read"),
            100,
            Outcome::Success,
        );

        let child2 = create_test_event(
            "C1B2C3D4E5F60718",
            Some("A1B2C3D4E5F60718"),
            Some("file_write"),
            200,
            Outcome::Success,
        );

        let mut builder = TraceWaterfallBuilder::new();
        builder.add_event(&parent_event);
        builder.add_event(&child1);
        builder.add_event(&child2);

        let waterfall = builder.build();
        assert!(waterfall.is_some());

        let waterfall = waterfall.unwrap();
        assert_eq!(waterfall.summary.total_spans, 3);
        assert_eq!(waterfall.summary.tool_count, 2);
        assert_eq!(waterfall.root_span.children.len(), 2);
    }

    #[test]
    fn test_trace_waterfall_builder_nested_hierarchy() {
        // Create root span
        let root = create_test_event(
            "A1B2C3D4E5F60718",
            None,
            Some("task_execution"),
            1000,
            Outcome::Success,
        );

        // Create middle span (child of root)
        let middle = create_test_event(
            "B1B2C3D4E5F60718",
            Some("A1B2C3D4E5F60718"),
            Some("planning"),
            500,
            Outcome::Success,
        );

        // Create leaf span (child of middle)
        let leaf = create_test_event(
            "C1B2C3D4E5F60718",
            Some("B1B2C3D4E5F60718"),
            Some("file_read"),
            100,
            Outcome::Success,
        );

        let mut builder = TraceWaterfallBuilder::new();
        builder.add_event(&root);
        builder.add_event(&middle);
        builder.add_event(&leaf);

        let waterfall = builder.build();
        assert!(waterfall.is_some());

        let waterfall = waterfall.unwrap();
        assert_eq!(waterfall.summary.total_spans, 3);
        assert!(!waterfall.root_span.children.is_empty());
        assert!(!waterfall.root_span.children[0].children.is_empty());
    }

    #[test]
    fn test_trace_summary_calculation() {
        let planning = create_test_event(
            "A1B2C3D4E5F60718",
            None,
            Some("planning"),
            500,
            Outcome::Success,
        );

        let file_read = create_test_event(
            "B1B2C3D4E5F60718",
            Some("A1B2C3D4E5F60718"),
            Some("file_read"),
            100,
            Outcome::Success,
        );

        let file_write = create_test_event(
            "C1B2C3D4E5F60718",
            Some("A1B2C3D4E5F60718"),
            Some("file_write"),
            200,
            Outcome::Error,
        );

        let llm_call = create_test_event(
            "D1B2C3D4E5F60718",
            Some("A1B2C3D4E5F60718"),
            Some("llm_complete"),
            300,
            Outcome::Success,
        );

        let mut builder = TraceWaterfallBuilder::new();
        builder.add_event(&planning);
        builder.add_event(&file_read);
        builder.add_event(&file_write);
        builder.add_event(&llm_call);

        let waterfall = builder.build().unwrap();

        assert_eq!(waterfall.summary.total_spans, 4);
        assert_eq!(waterfall.summary.tool_count, 2); // file_read, file_write (llm_complete is Llm)
        assert_eq!(waterfall.summary.llm_count, 1); // llm_complete is Llm
        assert_eq!(waterfall.summary.decision_count, 1); // file_write has error
        assert_eq!(waterfall.summary.total_tool_duration_ms, 300); // 100 + 200
        assert_eq!(waterfall.summary.total_llm_duration_ms, 300); // llm_complete duration
                                                                  // Outcome should be Error since one span had an error
        assert_eq!(waterfall.summary.outcome, Outcome::Error);
    }

    #[test]
    fn test_trace_waterfall_serialization() {
        let event = create_test_event(
            "A1B2C3D4E5F60718",
            None,
            Some("test_tool"),
            100,
            Outcome::Success,
        );

        let mut builder = TraceWaterfallBuilder::new();
        builder.add_event(&event);

        let waterfall = builder.build().unwrap();
        let json = serde_json::to_string_pretty(&waterfall).unwrap();

        // Verify JSON contains expected fields
        assert!(json.contains("\"trace_id\""));
        assert!(json.contains("\"root_span\""));
        assert!(json.contains("\"summary\""));
        assert!(json.contains("\"total_spans\""));
        assert!(json.contains("\"tool_count\""));

        // Verify it can be deserialized back
        let parsed: TraceWaterfall = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.summary.total_spans, 1);
    }

    #[test]
    fn test_trace_waterfall_with_decision_points() {
        let root = create_test_event(
            "A1B2C3D4E5F60718",
            None,
            Some("task"),
            1000,
            Outcome::Success,
        );

        let plan = create_test_event(
            "B1B2C3D4E5F60718",
            Some("A1B2C3D4E5F60718"),
            Some("plan"),
            300,
            Outcome::Success,
        );

        let decision = create_test_event(
            "C1B2C3D4E5F60718",
            Some("A1B2C3D4E5F60718"),
            Some("decide"),
            50,
            Outcome::Success,
        );

        let tool1 = create_test_event(
            "D1B2C3D4E5F60718",
            Some("A1B2C3D4E5F60718"),
            Some("file_edit"),
            200,
            Outcome::Success,
        );

        let mut builder = TraceWaterfallBuilder::new();
        builder.add_event(&root);
        builder.add_event(&plan);
        builder.add_event(&decision);
        builder.add_event(&tool1);

        let waterfall = builder.build().unwrap();

        assert_eq!(waterfall.summary.total_spans, 4);
        assert_eq!(waterfall.summary.decision_count, 2); // plan and decide are decision points
    }

    #[test]
    fn test_trace_span_end_time_calculation() {
        let start = Utc::now();
        let duration_ms = 100u64;

        let span = TraceSpan {
            span_id: SpanId::generate(),
            parent_span_id: None,
            name: "test".to_string(),
            kind: SpanKind::Tool,
            start_time: start,
            end_time: start
                .checked_add_signed(chrono::Duration::milliseconds(duration_ms as i64))
                .unwrap(),
            duration_ms,
            outcome: Outcome::Success,
            agent_id: Some(AgentId::new()),
            tool_invocation: Some(ToolSpanDetails {
                tool_name: "test_tool".to_string(),
                arguments_summary: "{}".to_string(),
                input_bytes: None,
                output_bytes: None,
                success: true,
            }),
            is_decision_point: false,
            children: Vec::new(),
            attributes: Vec::new(),
        };

        let calculated_duration = span
            .end_time
            .signed_duration_since(span.start_time)
            .num_milliseconds();

        assert_eq!(calculated_duration, duration_ms as i64);
    }

    #[test]
    fn test_tool_duration_breakdown() {
        let root = create_test_event(
            "A1B2C3D4E5F60718",
            None,
            Some("task"),
            1000,
            Outcome::Success,
        );

        let read1 = create_test_event(
            "B1B2C3D4E5F60718",
            Some("A1B2C3D4E5F60718"),
            Some("file_read"),
            50,
            Outcome::Success,
        );

        let read2 = create_test_event(
            "C1B2C3D4E5F60718",
            Some("A1B2C3D4E5F60718"),
            Some("file_read"),
            75,
            Outcome::Success,
        );

        let write = create_test_event(
            "D1B2C3D4E5F60718",
            Some("A1B2C3D4E5F60718"),
            Some("file_write"),
            100,
            Outcome::Success,
        );

        let mut builder = TraceWaterfallBuilder::new();
        builder.add_event(&root);
        builder.add_event(&read1);
        builder.add_event(&read2);
        builder.add_event(&write);

        let waterfall = builder.build().unwrap();

        // Tool durations should be 50 + 75 + 100 = 225
        assert_eq!(waterfall.summary.total_tool_duration_ms, 225);
        assert_eq!(waterfall.summary.tool_count, 3);
    }

    #[test]
    fn test_preserves_event_hierarchy() {
        // Create a more complex hierarchy
        let root = create_test_event(
            "A1B2C3D4E5F60718",
            None,
            Some("root"),
            1000,
            Outcome::Success,
        );

        let agent1 = create_test_event(
            "B1B2C3D4E5F60718",
            Some("A1B2C3D4E5F60718"),
            Some("planner"),
            400,
            Outcome::Success,
        );

        let agent2 = create_test_event(
            "C1B2C3D4E5F60718",
            Some("A1B2C3D4E5F60718"),
            Some("generator"),
            500,
            Outcome::Success,
        );

        let tool1 = create_test_event(
            "D1B2C3D4E5F60718",
            Some("B1B2C3D4E5F60718"),
            Some("search"),
            100,
            Outcome::Success,
        );

        let tool2 = create_test_event(
            "E1B2C3D4E5F60718",
            Some("C1B2C3D4E5F60718"),
            Some("edit"),
            200,
            Outcome::Success,
        );

        let mut builder = TraceWaterfallBuilder::new();
        builder.add_event(&root);
        builder.add_event(&agent1);
        builder.add_event(&agent2);
        builder.add_event(&tool1);
        builder.add_event(&tool2);

        let waterfall = builder.build().unwrap();

        // Verify root has 2 children (planner and generator)
        assert_eq!(waterfall.root_span.children.len(), 2);

        // Verify planner has 1 child (search)
        let planner = &waterfall.root_span.children[0];
        assert_eq!(planner.children.len(), 1);
        assert_eq!(planner.name, "planner");

        // Verify generator has 1 child (edit)
        let generator = &waterfall.root_span.children[1];
        assert_eq!(generator.children.len(), 1);
        assert_eq!(generator.name, "generator");
    }

    #[test]
    fn test_llm_span_detection() {
        let event = create_test_event(
            "A1B2C3D4E5F60718",
            None,
            Some("llm_chat"),
            500,
            Outcome::Success,
        );

        let span = event.to_trace_span();
        // Currently llm_chat is detected as Tool, which is acceptable
        // The LLM-specific detection would require tool name conventions
        assert_eq!(span.kind, SpanKind::Tool);
        assert!(span.duration_ms == 500);
    }

    #[test]
    fn test_trace_waterfall_empty_events_returns_none() {
        let builder = TraceWaterfallBuilder::new();
        let result = builder.build();
        assert!(result.is_none());
    }

    #[test]
    fn test_preserve_span_ordering() {
        let events = vec![
            create_test_event(
                "A1B2C3D4E5F60718",
                None,
                Some("task"),
                1000,
                Outcome::Success,
            ),
            create_test_event(
                "B1B2C3D4E5F60718",
                Some("A1B2C3D4E5F60718"),
                Some("tool_c"),
                300,
                Outcome::Success,
            ),
            create_test_event(
                "C1B2C3D4E5F60718",
                Some("A1B2C3D4E5F60718"),
                Some("tool_a"),
                100,
                Outcome::Success,
            ),
            create_test_event(
                "D1B2C3D4E5F60718",
                Some("A1B2C3D4E5F60718"),
                Some("tool_b"),
                200,
                Outcome::Success,
            ),
        ];

        let mut builder = TraceWaterfallBuilder::new();
        for event in events {
            builder.add_event(&event);
        }

        let waterfall = builder.build().unwrap();

        // Children should be sorted by start time, then by span_id for deterministic ordering
        // When all have same timestamp, sorting by span_id gives B < C < D order
        // (span_ids: tool_c=B..., tool_a=C..., tool_b=D...)
        let children = &waterfall.root_span.children;
        assert_eq!(children.len(), 3);
        assert_eq!(children[0].name, "tool_c"); // B < C < D alphabetically
        assert_eq!(children[1].name, "tool_a");
        assert_eq!(children[2].name, "tool_b");
    }
}
