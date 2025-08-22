use std::{collections::HashMap, fmt::Write as FmtWrite};

use serde::Serialize;

use crate::http::Method;

#[derive(Clone)]
pub struct RouteDetail {
    methods: Vec<Method>,
    guards: Vec<String>,
    is_resource: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum ResourceType {
    App,
    Scope,
    Resource,
}

#[derive(Debug, Clone)]
pub struct IntrospectionNode {
    pub kind: ResourceType,
    pub pattern: String,
    pub full_path: String,
    pub methods: Vec<Method>,
    pub guards: Vec<String>,
    pub children: Vec<IntrospectionNode>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IntrospectionReportItem {
    pub full_path: String,
    pub methods: Vec<String>,
    pub guards: Vec<String>,
}

impl IntrospectionNode {
    pub fn new(kind: ResourceType, pattern: String, full_path: String) -> Self {
        IntrospectionNode {
            kind,
            pattern,
            full_path,
            methods: Vec::new(),
            guards: Vec::new(),
            children: Vec::new(),
        }
    }
}

impl From<&IntrospectionNode> for Vec<IntrospectionReportItem> {
    fn from(node: &IntrospectionNode) -> Self {
        fn collect_report_items(
            node: &IntrospectionNode,
            parent_path: &str,
            report_items: &mut Vec<IntrospectionReportItem>,
        ) {
            let full_path = if parent_path.is_empty() {
                node.pattern.clone()
            } else {
                format!(
                    "{}/{}",
                    parent_path.trim_end_matches('/'),
                    node.pattern.trim_start_matches('/')
                )
            };

            if !node.methods.is_empty() || !node.guards.is_empty() {
                let filtered_guards: Vec<String> = node
                    .guards
                    .iter()
                    .filter(|guard| !node.methods.iter().any(|m| m.to_string() == **guard))
                    .cloned()
                    .collect();

                report_items.push(IntrospectionReportItem {
                    full_path: full_path.clone(),
                    methods: node.methods.iter().map(|m| m.to_string()).collect(),
                    guards: filtered_guards,
                });
            }

            for child in &node.children {
                collect_report_items(child, &full_path, report_items);
            }
        }

        let mut report_items = Vec::new();
        collect_report_items(node, "/", &mut report_items);
        report_items
    }
}

#[derive(Clone, Default)]
pub struct IntrospectionCollector {
    details: HashMap<String, RouteDetail>,
}

impl IntrospectionCollector {
    pub fn new() -> Self {
        Self {
            details: HashMap::new(),
        }
    }

    pub fn register_pattern_detail(
        &mut self,
        full_path: String,
        methods: Vec<Method>,
        guards: Vec<String>,
        is_resource: bool,
    ) {
        self.details
            .entry(full_path)
            .and_modify(|d| {
                update_unique(&mut d.methods, &methods);
                update_unique(&mut d.guards, &guards);
                if !d.is_resource && is_resource {
                    d.is_resource = true;
                }
            })
            .or_insert(RouteDetail {
                methods,
                guards,
                is_resource,
            });
    }

    pub fn finalize(&mut self) -> IntrospectionTree {
        let detail_registry = std::mem::take(&mut self.details);
        let mut root = IntrospectionNode::new(ResourceType::App, "".into(), "".into());

        for (full_path, _) in detail_registry.iter() {
            let parts: Vec<&str> = full_path.split('/').collect();
            let mut current_node = &mut root;

            for (i, part) in parts.iter().enumerate() {
                let existing_child_index = current_node
                    .children
                    .iter()
                    .position(|n| n.pattern == *part);

                let child_index = if let Some(idx) = existing_child_index {
                    idx
                } else {
                    let child_full_path = parts[..=i].join("/");
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

                if let ResourceType::Resource = current_node.kind {
                    if let Some(detail) = detail_registry.get(&current_node.full_path) {
                        update_unique(&mut current_node.methods, &detail.methods);
                        update_unique(&mut current_node.guards, &detail.guards);
                    }
                }
            }
        }

        IntrospectionTree { root }
    }
}

#[derive(Clone)]
pub struct IntrospectionTree {
    pub root: IntrospectionNode,
}

impl IntrospectionTree {
    pub fn report_as_text(&self) -> String {
        let report_items: Vec<IntrospectionReportItem> = (&self.root).into();

        let mut buf = String::new();
        for item in report_items {
            writeln!(
                buf,
                "{} => Methods: {:?} | Guards: {:?}",
                item.full_path, item.methods, item.guards
            )
            .unwrap();
        }

        buf
    }

    pub fn report_as_json(&self) -> String {
        let report_items: Vec<IntrospectionReportItem> = (&self.root).into();
        serde_json::to_string_pretty(&report_items).unwrap()
    }
}

fn update_unique<T: Clone + PartialEq>(existing: &mut Vec<T>, new_items: &[T]) {
    for item in new_items {
        if !existing.contains(item) {
            existing.push(item.clone());
        }
    }
}
