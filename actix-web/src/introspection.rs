//! Experimental route introspection helpers.
//!
//! Enabled with the `experimental-introspection` feature.
//!
//! What it reports:
//! - Configured routes with their patterns, method guards, guard details, and resource metadata
//!   (`resource_name`, `resource_type`, `scope_depth`).
//! - Reachability hints for routes that may be shadowed by registration order or conflicting
//!   method guards.
//! - External resources (used only for URL generation) in a separate report, including the scope
//!   path where they were registered. External resources never participate in request routing.
//!
//! Notes:
//! - Method lists are best-effort and derived only from explicit method guards; an empty list means
//!   the route matches any method.
//! - Reachability hints are best-effort and should be treated as diagnostics, not a hard guarantee.
//!
//! This feature is intended for local/non-production use. Avoid exposing introspection endpoints
//! in production, since reports can include sensitive configuration details.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as FmtWrite,
};

use serde::Serialize;

use crate::{
    dev::ResourceDef,
    guard::{Guard, GuardDetail},
    http::Method,
};

#[derive(Clone)]
pub struct RouteDetail {
    methods: Vec<Method>,
    guards: Vec<String>,
    guard_details: Vec<GuardReport>,
    patterns: Vec<String>,
    resource_name: Option<String>,
    is_resource: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GuardReport {
    pub name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<GuardDetailReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GuardDetailReport {
    HttpMethods { methods: Vec<String> },
    Headers { headers: Vec<HeaderReport> },
    Generic { value: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HeaderReport {
    pub name: String,
    pub value: String,
}

/// A report item for an external resource configured for URL generation.
///
/// `origin_scope` is the scope path where the external resource was registered. It is informational
/// only and does not affect URL generation or routing; external resources are always global.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExternalResourceReportItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub patterns: Vec<String>,
    pub origin_scope: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegistrationKind {
    Service,
    Route,
}

#[derive(Clone)]
struct Registration {
    order: usize,
    kind: RegistrationKind,
    scope_id: Option<usize>,
    parent_scope_id: Option<usize>,
    full_path: String,
    is_prefix: bool,
    methods: Vec<Method>,
    guards: Vec<String>,
}

#[derive(Clone)]
struct ShadowingContext {
    path: String,
    order: usize,
}

/// Node type within an introspection tree.
#[derive(Debug, Clone, Copy)]
pub enum ResourceType {
    /// The application root.
    App,
    /// A scope/prefix path.
    Scope,
    /// A resource (route) path.
    Resource,
}

fn resource_type_label(kind: ResourceType) -> &'static str {
    match kind {
        ResourceType::App => "app",
        ResourceType::Scope => "scope",
        ResourceType::Resource => "resource",
    }
}

/// A node in the introspection tree.
#[derive(Debug, Clone)]
pub struct IntrospectionNode {
    /// The node's classification.
    pub kind: ResourceType,
    /// The path segment used for this node.
    pub pattern: String,
    /// The full path for this node.
    pub full_path: String,
    /// HTTP methods derived from explicit method guards.
    pub methods: Vec<Method>,
    /// Guard names attached to this node.
    pub guards: Vec<String>,
    /// Structured guard details, when available.
    pub guard_details: Vec<GuardReport>,
    /// Resource name, when configured.
    pub resource_name: Option<String>,
    /// Original patterns used for this resource.
    pub patterns: Vec<String>,
    /// Child nodes under this prefix.
    pub children: Vec<IntrospectionNode>,
    /// True if the node might be unreachable at runtime.
    pub potentially_unreachable: bool,
    /// Reasons for potential unreachability.
    pub reachability_notes: Vec<String>,
}

/// A flattened report item for a route.
#[derive(Debug, Clone, Serialize)]
pub struct IntrospectionReportItem {
    /// Full path for the route.
    pub full_path: String,
    /// Methods derived from explicit method guards.
    ///
    /// An empty list indicates the route matches any method.
    pub methods: Vec<String>,
    /// Guard names attached to the route.
    pub guards: Vec<String>,
    /// Structured guard details, when available.
    ///
    /// Includes method guards even if `guards` filters them out for readability.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub guards_detail: Vec<GuardReport>,
    /// Resource name, when configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_name: Option<String>,
    /// Original patterns used for this resource.
    ///
    /// These are raw ResourceDef patterns (may be relative to a scope), not expanded full paths.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub patterns: Vec<String>,
    /// The type of node represented by the report item.
    pub resource_type: String,
    /// Depth within the scope tree (root = 0).
    pub scope_depth: usize,
    /// True if the route might be unreachable at runtime.
    #[serde(skip_serializing_if = "is_false")]
    pub potentially_unreachable: bool,
    /// Reasons for potential unreachability.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reachability_notes: Vec<String>,
}

impl IntrospectionNode {
    pub fn new(kind: ResourceType, pattern: String, full_path: String) -> Self {
        IntrospectionNode {
            kind,
            pattern,
            full_path,
            methods: Vec::new(),
            guards: Vec::new(),
            guard_details: Vec::new(),
            resource_name: None,
            patterns: Vec::new(),
            children: Vec::new(),
            potentially_unreachable: false,
            reachability_notes: Vec::new(),
        }
    }
}

impl From<&IntrospectionNode> for Vec<IntrospectionReportItem> {
    fn from(node: &IntrospectionNode) -> Self {
        fn collect_report_items(
            node: &IntrospectionNode,
            report_items: &mut Vec<IntrospectionReportItem>,
            depth: usize,
        ) {
            let include_node = matches!(node.kind, ResourceType::Resource)
                || !node.methods.is_empty()
                || !node.guards.is_empty()
                || node.potentially_unreachable;

            if include_node {
                let method_names = node
                    .methods
                    .iter()
                    .map(|m| m.to_string())
                    .collect::<Vec<_>>();
                let filtered_guards = filter_guard_names(&node.guards, &node.methods);

                report_items.push(IntrospectionReportItem {
                    full_path: node.full_path.clone(),
                    methods: method_names,
                    guards: filtered_guards,
                    guards_detail: node.guard_details.clone(),
                    resource_name: node.resource_name.clone(),
                    patterns: node.patterns.clone(),
                    resource_type: resource_type_label(node.kind).to_string(),
                    scope_depth: depth,
                    potentially_unreachable: node.potentially_unreachable,
                    reachability_notes: node.reachability_notes.clone(),
                });
            }

            for child in &node.children {
                collect_report_items(child, report_items, depth + 1);
            }
        }

        let mut report_items = Vec::new();
        collect_report_items(node, &mut report_items, 0);
        report_items
    }
}

/// Collects route details during app configuration.
#[derive(Clone, Default)]
pub struct IntrospectionCollector {
    details: BTreeMap<String, RouteDetail>,
    registrations: Vec<Registration>,
    externals: Vec<ExternalResourceReportItem>,
    next_registration_order: usize,
    next_scope_id: usize,
}

impl IntrospectionCollector {
    /// Creates a new, empty collector.
    pub fn new() -> Self {
        Self {
            details: BTreeMap::new(),
            registrations: Vec::new(),
            externals: Vec::new(),
            next_registration_order: 0,
            next_scope_id: 0,
        }
    }

    pub fn next_scope_id(&mut self) -> usize {
        let scope_id = self.next_scope_id;
        self.next_scope_id += 1;
        scope_id
    }

    pub fn register_service(
        &mut self,
        full_path: String,
        methods: Vec<Method>,
        guards: Vec<String>,
        guard_details: Vec<GuardReport>,
        patterns: Vec<String>,
        resource_name: Option<String>,
        is_resource: bool,
        is_prefix: bool,
        scope_id: Option<usize>,
        parent_scope_id: Option<usize>,
    ) {
        let full_path = normalize_path(&full_path);

        self.register_pattern_detail(
            full_path.clone(),
            methods.clone(),
            guards.clone(),
            guard_details.clone(),
            patterns.clone(),
            resource_name.clone(),
            is_resource,
        );

        self.registrations.push(Registration {
            order: self.next_registration_order,
            kind: RegistrationKind::Service,
            scope_id,
            parent_scope_id,
            full_path,
            is_prefix,
            methods,
            guards,
        });
        self.next_registration_order += 1;
    }

    pub fn register_route(
        &mut self,
        full_path: String,
        methods: Vec<Method>,
        guards: Vec<String>,
        guard_details: Vec<GuardReport>,
        patterns: Vec<String>,
        resource_name: Option<String>,
        scope_id: Option<usize>,
    ) {
        let full_path = normalize_path(&full_path);

        self.register_pattern_detail(
            full_path.clone(),
            methods.clone(),
            guards.clone(),
            guard_details.clone(),
            patterns.clone(),
            resource_name.clone(),
            true,
        );

        self.registrations.push(Registration {
            order: self.next_registration_order,
            kind: RegistrationKind::Route,
            scope_id,
            parent_scope_id: None,
            full_path,
            is_prefix: false,
            methods,
            guards,
        });
        self.next_registration_order += 1;
    }

    pub fn register_external(&mut self, rdef: &ResourceDef, origin_scope: &str) {
        let report = external_report_from_rdef(rdef, origin_scope);

        if let Some(name) = report.name.as_deref() {
            if let Some(existing) = self
                .externals
                .iter_mut()
                .find(|item| item.name.as_deref() == Some(name))
            {
                *existing = report;
                return;
            }
        }

        if !self.externals.contains(&report) {
            self.externals.push(report);
        }
    }

    /// Registers details for a route pattern.
    pub fn register_pattern_detail(
        &mut self,
        full_path: String,
        methods: Vec<Method>,
        guards: Vec<String>,
        guard_details: Vec<GuardReport>,
        patterns: Vec<String>,
        resource_name: Option<String>,
        is_resource: bool,
    ) {
        let full_path = normalize_path(&full_path);

        self.details
            .entry(full_path)
            .and_modify(|d| {
                update_unique(&mut d.methods, &methods);
                update_unique(&mut d.guards, &guards);
                merge_guard_reports(&mut d.guard_details, &guard_details);
                update_unique(&mut d.patterns, &patterns);
                if d.resource_name.is_none() {
                    d.resource_name = resource_name.clone();
                }
                if !d.is_resource && is_resource {
                    d.is_resource = true;
                }
            })
            .or_insert(RouteDetail {
                methods,
                guards,
                guard_details,
                patterns,
                resource_name,
                is_resource,
            });
    }

    /// Produces the finalized introspection tree.
    pub fn finalize(&mut self) -> IntrospectionTree {
        let detail_registry = std::mem::take(&mut self.details);
        let registrations = std::mem::take(&mut self.registrations);
        let externals = std::mem::take(&mut self.externals);
        let mut root = IntrospectionNode::new(ResourceType::App, "".into(), "".into());

        for (full_path, _) in detail_registry.iter() {
            let parts = split_path_segments(full_path);
            let mut current_node = &mut root;
            let mut assembled = String::new();

            for part in parts.iter() {
                if assembled.is_empty() {
                    assembled.push('/');
                    assembled.push_str(part);
                } else {
                    assembled.push('/');
                    assembled.push_str(part);
                }

                let child_full_path = assembled.clone();
                let existing_child_index = current_node
                    .children
                    .iter()
                    .position(|n| n.pattern == *part);

                let child_index = if let Some(idx) = existing_child_index {
                    idx
                } else {
                    let kind = if detail_registry
                        .get(&child_full_path)
                        .is_some_and(|d| d.is_resource)
                    {
                        ResourceType::Resource
                    } else {
                        ResourceType::Scope
                    };
                    let new_node = IntrospectionNode::new(kind, part.to_string(), child_full_path);
                    current_node.children.push(new_node);
                    current_node.children.len() - 1
                };

                current_node = &mut current_node.children[child_index];

                if let Some(detail) = detail_registry.get(&current_node.full_path) {
                    update_unique(&mut current_node.methods, &detail.methods);
                    update_unique(&mut current_node.guards, &detail.guards);
                    merge_guard_reports(&mut current_node.guard_details, &detail.guard_details);
                    update_unique(&mut current_node.patterns, &detail.patterns);
                    if current_node.resource_name.is_none() {
                        current_node.resource_name = detail.resource_name.clone();
                    }
                }
            }
        }

        let reachability = analyze_reachability(&registrations);
        apply_reachability(&mut root, &reachability);

        IntrospectionTree { root, externals }
    }
}

/// The finalized introspection tree.
#[derive(Clone)]
pub struct IntrospectionTree {
    /// Root node of the tree.
    pub root: IntrospectionNode,
    /// External resources configured for URL generation.
    pub externals: Vec<ExternalResourceReportItem>,
}

impl IntrospectionTree {
    /// Returns a formatted, human-readable report.
    pub fn report_as_text(&self) -> String {
        warn_release_mode_once();
        let report_items: Vec<IntrospectionReportItem> = (&self.root).into();

        let mut buf = String::new();
        for item in report_items {
            let full_path = sanitize_text(&item.full_path);
            let methods = item
                .methods
                .iter()
                .map(|method| sanitize_text(method))
                .collect::<Vec<_>>();
            let guards = item
                .guards
                .iter()
                .map(|guard| sanitize_text(guard))
                .collect::<Vec<_>>();
            writeln!(
                buf,
                "{} => Methods: {:?} | Guards: {:?}{}",
                full_path,
                methods,
                guards,
                format_reachability(&item)
            )
            .unwrap();
        }

        buf
    }

    /// Returns a JSON report of configured routes.
    pub fn report_as_json(&self) -> String {
        warn_release_mode_once();
        let report_items: Vec<IntrospectionReportItem> = (&self.root).into();
        serde_json::to_string_pretty(&report_items).unwrap()
    }

    /// Returns a JSON report of external resources.
    pub fn report_externals_as_json(&self) -> String {
        warn_release_mode_once();
        serde_json::to_string_pretty(&self.externals).unwrap()
    }
}

pub(crate) fn guard_reports_from_iter<'a, I>(guards: I) -> Vec<GuardReport>
where
    I: IntoIterator<Item = &'a Box<dyn Guard>>,
{
    guards
        .into_iter()
        .map(|guard| {
            let mut details = Vec::new();
            if let Some(guard_details) = guard.details() {
                for detail in guard_details {
                    merge_guard_detail_reports(&mut details, detail.into());
                }
            }
            GuardReport {
                name: guard.name(),
                details,
            }
        })
        .collect()
}

impl From<GuardDetail> for GuardDetailReport {
    fn from(detail: GuardDetail) -> Self {
        match detail {
            GuardDetail::HttpMethods(methods) => GuardDetailReport::HttpMethods { methods },
            GuardDetail::Headers(headers) => GuardDetailReport::Headers {
                headers: headers
                    .into_iter()
                    .map(|(name, value)| HeaderReport { name, value })
                    .collect(),
            },
            GuardDetail::Generic(value) => GuardDetailReport::Generic { value },
        }
    }
}

pub(crate) fn external_report_from_rdef(
    rdef: &ResourceDef,
    origin_scope: &str,
) -> ExternalResourceReportItem {
    ExternalResourceReportItem {
        name: rdef.name().map(|name| name.to_string()),
        patterns: rdef
            .pattern_iter()
            .map(|pattern| pattern.to_string())
            .collect(),
        origin_scope: normalize_path(origin_scope),
    }
}

pub(crate) fn expand_patterns(prefix: &str, rdef: &ResourceDef) -> Vec<String> {
    let mut full_paths = Vec::new();

    if prefix.is_empty() {
        for pat in rdef.pattern_iter() {
            full_paths.push(normalize_path(pat));
        }

        return full_paths;
    }

    let joined = ResourceDef::root_prefix(prefix).join(rdef);

    for pat in joined.pattern_iter() {
        full_paths.push(normalize_path(pat));
    }

    full_paths
}

fn analyze_reachability(registrations: &[Registration]) -> BTreeMap<String, Vec<String>> {
    let shadowed_scopes = shadowed_scope_context(registrations);
    let shadowed_routes = shadowed_route_context(registrations);

    let mut notes_by_path: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for reg in registrations {
        let mut notes = Vec::new();

        if let Some(scope_id) = reg.scope_id {
            if let Some(context) = shadowed_scopes.get(&scope_id) {
                notes.push("shadowed_by_scope".to_string());
                notes.push(format!("shadowed_by_path:{}", context.path));
                notes.push(format!("shadowed_by_order:{}", context.order));
            }
        }

        if reg.kind == RegistrationKind::Route {
            if let Some(context) = shadowed_routes.get(&(reg.scope_id, reg.full_path.clone())) {
                notes.push("shadowed_by_route".to_string());
                notes.push(format!("shadowed_by_path:{}", context.path));
                notes.push(format!("shadowed_by_order:{}", context.order));
            }

            if has_conflicting_methods(&reg.methods, &reg.guards) {
                notes.push("conflicting_method_guards".to_string());
            }
        }

        if !notes.is_empty() {
            let entry = notes_by_path.entry(reg.full_path.clone()).or_default();
            for note in notes {
                entry.insert(note);
            }
        }
    }

    notes_by_path
        .into_iter()
        .map(|(path, notes)| (path, notes.into_iter().collect()))
        .collect()
}

fn shadowed_scope_context(registrations: &[Registration]) -> BTreeMap<usize, ShadowingContext> {
    let mut groups: BTreeMap<(Option<usize>, String), Vec<&Registration>> = BTreeMap::new();

    for reg in registrations {
        if reg.kind != RegistrationKind::Service || !reg.is_prefix {
            continue;
        }

        if reg.scope_id.is_none() {
            continue;
        }

        groups
            .entry((reg.parent_scope_id, reg.full_path.clone()))
            .or_default()
            .push(reg);
    }

    let mut shadowed = BTreeMap::new();

    for regs in groups.values_mut() {
        regs.sort_by_key(|reg| reg.order);

        let mut shadowing_reg = None;

        for reg in regs.iter() {
            if matches_all(&reg.methods, &reg.guards) {
                shadowing_reg = Some(*reg);
                break;
            }
        }

        if let Some(shadowing) = shadowing_reg {
            for reg in regs.iter() {
                if reg.order > shadowing.order {
                    let scope_id = reg.scope_id.expect("scope_id must exist");
                    shadowed.insert(
                        scope_id,
                        ShadowingContext {
                            path: shadowing.full_path.clone(),
                            order: shadowing.order,
                        },
                    );
                }
            }
        }
    }

    shadowed
}

fn shadowed_route_context(
    registrations: &[Registration],
) -> BTreeMap<(Option<usize>, String), ShadowingContext> {
    let mut groups: BTreeMap<(Option<usize>, String), Vec<&Registration>> = BTreeMap::new();

    for reg in registrations {
        if reg.kind != RegistrationKind::Route {
            continue;
        }

        groups
            .entry((reg.scope_id, reg.full_path.clone()))
            .or_default()
            .push(reg);
    }

    let mut shadowed = BTreeMap::new();

    for (key, regs) in groups {
        let mut regs = regs;
        regs.sort_by_key(|reg| reg.order);

        for idx in 1..regs.len() {
            let current = regs[idx];
            let current_methods = method_set(&current.methods);

            if !guards_only_methods(&current.guards, &current.methods) {
                continue;
            }

            let mut shadowing_reg = None;

            for earlier in &regs[..idx] {
                if !guards_only_methods(&earlier.guards, &earlier.methods) {
                    continue;
                }

                if earlier.methods.is_empty() {
                    shadowing_reg = Some(*earlier);
                    break;
                }

                let earlier_methods = method_set(&earlier.methods);
                if !current_methods.is_empty() && current_methods.is_subset(&earlier_methods) {
                    shadowing_reg = Some(*earlier);
                    break;
                }
            }

            if let Some(reg) = shadowing_reg {
                shadowed.insert(
                    key.clone(),
                    ShadowingContext {
                        path: reg.full_path.clone(),
                        order: reg.order,
                    },
                );
                break;
            }
        }
    }

    shadowed
}

fn apply_reachability(root: &mut IntrospectionNode, notes: &BTreeMap<String, Vec<String>>) {
    fn apply(node: &mut IntrospectionNode, notes: &BTreeMap<String, Vec<String>>) {
        if let Some(node_notes) = notes.get(&node.full_path) {
            node.potentially_unreachable = true;
            node.reachability_notes = node_notes.clone();
        }

        for child in &mut node.children {
            apply(child, notes);
        }
    }

    apply(root, notes);
}

fn normalize_path(path: &str) -> String {
    if path.is_empty() {
        return "/".to_string();
    }

    if path.starts_with('/') {
        path.to_string()
    } else {
        let mut buf = String::with_capacity(path.len() + 1);
        buf.push('/');
        buf.push_str(path);
        buf
    }
}

fn split_path_segments(path: &str) -> Vec<&str> {
    let trimmed = path.strip_prefix('/').unwrap_or(path);

    if trimmed.is_empty() {
        return vec![""];
    }

    trimmed.split('/').collect()
}

fn matches_all(methods: &[Method], guards: &[String]) -> bool {
    methods.is_empty() && filter_guard_names(guards, methods).is_empty()
}

fn guards_only_methods(guards: &[String], methods: &[Method]) -> bool {
    filter_guard_names(guards, methods).is_empty()
}

fn has_conflicting_methods(methods: &[Method], guards: &[String]) -> bool {
    let method_names = method_set(methods);
    if method_names.len() <= 1 {
        return false;
    }

    let has_any = guards.iter().any(|name| name.starts_with("AnyGuard("));
    let has_all = guards.iter().any(|name| name.starts_with("AllGuard("));

    if has_all {
        return true;
    }

    !has_any
}

fn method_set(methods: &[Method]) -> BTreeSet<String> {
    methods.iter().map(|m| m.to_string()).collect()
}

fn filter_guard_names(guards: &[String], methods: &[Method]) -> Vec<String> {
    let method_names = method_set(methods);
    guards
        .iter()
        .filter(|guard| !method_names.iter().any(|method| method == *guard))
        .cloned()
        .collect()
}

fn merge_guard_reports(existing: &mut Vec<GuardReport>, incoming: &[GuardReport]) {
    for report in incoming {
        if let Some(existing_report) = existing.iter_mut().find(|r| r.name == report.name) {
            for detail in &report.details {
                merge_guard_detail_reports(&mut existing_report.details, detail.clone());
            }
        } else {
            existing.push(report.clone());
        }
    }
}

fn merge_guard_detail_reports(existing: &mut Vec<GuardDetailReport>, incoming: GuardDetailReport) {
    match incoming {
        GuardDetailReport::HttpMethods { methods } => {
            if let Some(existing_methods) = existing.iter_mut().find_map(|detail| {
                if let GuardDetailReport::HttpMethods { methods } = detail {
                    Some(methods)
                } else {
                    None
                }
            }) {
                update_unique(existing_methods, &methods);
            } else {
                existing.push(GuardDetailReport::HttpMethods { methods });
            }
        }
        GuardDetailReport::Headers { headers } => {
            if let Some(existing_headers) = existing.iter_mut().find_map(|detail| {
                if let GuardDetailReport::Headers { headers } = detail {
                    Some(headers)
                } else {
                    None
                }
            }) {
                update_unique(existing_headers, &headers);
            } else {
                existing.push(GuardDetailReport::Headers { headers });
            }
        }
        GuardDetailReport::Generic { value } => {
            let detail = GuardDetailReport::Generic { value };
            if !existing.contains(&detail) {
                existing.push(detail);
            }
        }
    }
}

fn update_unique<T: Clone + PartialEq>(existing: &mut Vec<T>, new_items: &[T]) {
    for item in new_items {
        if !existing.contains(item) {
            existing.push(item.clone());
        }
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn format_reachability(item: &IntrospectionReportItem) -> String {
    if !item.potentially_unreachable {
        return String::new();
    }

    if item.reachability_notes.is_empty() {
        " | PotentiallyUnreachable".to_string()
    } else {
        format!(
            " | PotentiallyUnreachable | Notes: {:?}",
            item.reachability_notes
        )
    }
}

fn sanitize_text(value: &str) -> String {
    // Escape control characters to keep the text report format stable in logs/terminals.
    let mut buf = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_control() {
            let code = ch as u32;
            if code <= 0xFF {
                write!(buf, "\\x{:02x}", code).unwrap();
            } else {
                write!(buf, "\\u{{{:x}}}", code).unwrap();
            }
        } else {
            buf.push(ch);
        }
    }
    buf
}

fn warn_release_mode_once() {
    #[cfg(not(debug_assertions))]
    {
        use std::sync::Once;

        static WARN_ONCE: Once = Once::new();
        WARN_ONCE.call_once(|| {
            log::warn!(
                "experimental-introspection is intended for local/non-production use; \
avoid exposing introspection endpoints in production"
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_includes_resources_without_methods() {
        let mut collector = IntrospectionCollector::new();
        collector.register_route(
            "/no-guards".to_string(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
        );
        let tree = collector.finalize();
        let items: Vec<IntrospectionReportItem> = (&tree.root).into();

        let item = items
            .iter()
            .find(|item| item.full_path == "/no-guards")
            .expect("missing resource without guards");

        assert!(item.methods.is_empty());
        assert!(item.guards.is_empty());
        assert_eq!(item.resource_type, "resource");
        assert!(!item.potentially_unreachable);
        assert!(item.reachability_notes.is_empty());
    }

    #[test]
    fn report_includes_guard_details_and_metadata() {
        let mut collector = IntrospectionCollector::new();
        let guard_details = vec![GuardReport {
            name: "Header(accept, text/plain)".to_string(),
            details: vec![GuardDetailReport::Headers {
                headers: vec![HeaderReport {
                    name: "accept".to_string(),
                    value: "text/plain".to_string(),
                }],
            }],
        }];

        collector.register_route(
            "/meta".to_string(),
            vec![Method::GET],
            vec!["Header(accept, text/plain)".to_string()],
            guard_details,
            vec!["/meta".to_string()],
            Some("meta-resource".to_string()),
            None,
        );

        let tree = collector.finalize();
        let items: Vec<IntrospectionReportItem> = (&tree.root).into();

        let item = items
            .iter()
            .find(|item| item.full_path == "/meta")
            .expect("missing metadata route");

        assert_eq!(item.resource_name.as_deref(), Some("meta-resource"));
        assert!(item.patterns.contains(&"/meta".to_string()));
        assert_eq!(item.resource_type, "resource");
        assert_eq!(item.scope_depth, 1);
        assert_eq!(item.guards_detail.len(), 1);
    }

    #[test]
    fn expand_patterns_handles_scope_paths() {
        let empty = ResourceDef::new("");
        let slash = ResourceDef::new("/");

        assert_eq!(expand_patterns("/app", &empty), vec!["/app"]);
        assert_eq!(expand_patterns("/app", &slash), vec!["/app/"]);
        assert_eq!(expand_patterns("/app/", &empty), vec!["/app/"]);
        assert_eq!(expand_patterns("/app/", &slash), vec!["/app//"]);
    }

    #[test]
    fn expand_patterns_handles_multi_patterns() {
        let rdef = ResourceDef::new(["/a", "/b"]);
        assert_eq!(expand_patterns("/api", &rdef), vec!["/api/a", "/api/b"]);
    }

    #[test]
    fn conflicting_method_guards_mark_unreachable() {
        let mut collector = IntrospectionCollector::new();
        collector.register_route(
            "/all-guard".to_string(),
            vec![Method::GET, Method::POST],
            vec!["AllGuard(GET, POST)".to_string()],
            Vec::new(),
            Vec::new(),
            None,
            None,
        );
        let tree = collector.finalize();
        let items: Vec<IntrospectionReportItem> = (&tree.root).into();

        let item = items
            .iter()
            .find(|item| item.full_path == "/all-guard")
            .expect("missing route");

        assert!(item.potentially_unreachable);
        assert!(item
            .reachability_notes
            .contains(&"conflicting_method_guards".to_string()));
    }

    #[test]
    fn shadowed_scopes_mark_routes() {
        let mut collector = IntrospectionCollector::new();

        let scope_a = collector.next_scope_id();
        collector.register_service(
            "/extra".to_string(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            true,
            true,
            Some(scope_a),
            None,
        );
        collector.register_route(
            "/extra/ping".to_string(),
            vec![Method::GET],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            Some(scope_a),
        );

        let scope_b = collector.next_scope_id();
        collector.register_service(
            "/extra".to_string(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            true,
            true,
            Some(scope_b),
            None,
        );
        collector.register_route(
            "/extra/ping".to_string(),
            vec![Method::POST],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            Some(scope_b),
        );

        let tree = collector.finalize();
        let items: Vec<IntrospectionReportItem> = (&tree.root).into();

        let item = items
            .iter()
            .find(|item| item.full_path == "/extra/ping")
            .expect("missing route");

        assert!(item.potentially_unreachable);
        assert!(item
            .reachability_notes
            .contains(&"shadowed_by_scope".to_string()));
        assert!(item
            .reachability_notes
            .contains(&"shadowed_by_path:/extra".to_string()));
        assert!(item
            .reachability_notes
            .contains(&"shadowed_by_order:0".to_string()));
    }

    #[test]
    fn shadowed_routes_include_context() {
        let mut collector = IntrospectionCollector::new();

        collector.register_route(
            "/shadow".to_string(),
            vec![Method::GET],
            vec!["GET".to_string()],
            Vec::new(),
            Vec::new(),
            None,
            None,
        );
        collector.register_route(
            "/shadow".to_string(),
            vec![Method::GET],
            vec!["GET".to_string()],
            Vec::new(),
            Vec::new(),
            None,
            None,
        );

        let tree = collector.finalize();
        let items: Vec<IntrospectionReportItem> = (&tree.root).into();

        let item = items
            .iter()
            .find(|item| item.full_path == "/shadow")
            .expect("missing route");

        assert!(item.potentially_unreachable);
        assert!(item
            .reachability_notes
            .contains(&"shadowed_by_route".to_string()));
        assert!(item
            .reachability_notes
            .contains(&"shadowed_by_path:/shadow".to_string()));
        assert!(item
            .reachability_notes
            .contains(&"shadowed_by_order:0".to_string()));
    }
}
