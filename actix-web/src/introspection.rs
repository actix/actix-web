use std::{
    collections::HashMap,
    fmt::Write as FmtWrite,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock,
    },
    thread,
};

use serde::Serialize;

use crate::http::Method;

static REGISTRY: OnceLock<Mutex<IntrospectionNode>> = OnceLock::new();
static DETAIL_REGISTRY: OnceLock<Mutex<HashMap<String, RouteDetail>>> = OnceLock::new();
static DESIGNATED_THREAD: OnceLock<thread::ThreadId> = OnceLock::new();
static IS_INITIALIZED: AtomicBool = AtomicBool::new(false);

fn initialize_registry() {
    REGISTRY.get_or_init(|| {
        Mutex::new(IntrospectionNode::new(
            ResourceType::App,
            "".into(),
            "".into(),
        ))
    });
}

fn get_registry() -> &'static Mutex<IntrospectionNode> {
    REGISTRY.get().expect("Registry not initialized")
}

fn initialize_detail_registry() {
    DETAIL_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()));
}

fn get_detail_registry() -> &'static Mutex<HashMap<String, RouteDetail>> {
    DETAIL_REGISTRY
        .get()
        .expect("Detail registry not initialized")
}

fn is_designated_thread() -> bool {
    let current_id = thread::current().id();
    DESIGNATED_THREAD.get_or_init(|| {
        IS_INITIALIZED.store(true, Ordering::SeqCst);
        current_id // Assign the first thread that calls this function
    });

    *DESIGNATED_THREAD.get().unwrap() == current_id
}

#[derive(Clone)]
pub struct RouteDetail {
    methods: Vec<Method>,
    guards: Vec<String>,
    is_resource: bool, // Indicates if this detail is for a final resource endpoint
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
                // Filter guards that are already represented in methods
                let filtered_guards: Vec<String> = node
                    .guards
                    .iter()
                    .filter(|guard| {
                        !node
                            .methods
                            .iter()
                            .any(|method| method.to_string() == **guard)
                    })
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

pub(crate) fn finalize_registry() {
    if !is_designated_thread() {
        return;
    }

    initialize_registry();
    initialize_detail_registry();

    let detail_registry = get_detail_registry().lock().unwrap();
    let mut root = IntrospectionNode::new(ResourceType::App, "".into(), "".into());

    // Build the introspection tree directly from the detail registry
    for (full_path, _detail) in detail_registry.iter() {
        let parts: Vec<&str> = full_path.split('/').collect();
        let mut current_node = &mut root;

        for (i, part) in parts.iter().enumerate() {
            // Find the index of the existing child
            let existing_child_index = current_node
                .children
                .iter()
                .position(|n| n.pattern == *part);

            let child_index = if let Some(child_index) = existing_child_index {
                child_index
            } else {
                // If it doesn't exist, create a new node and get its index
                let child_full_path = parts[..=i].join("/");
                // Determine the kind based on whether this path exists as a resource in the detail registry
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

            // Get a mutable reference to the child node
            current_node = &mut current_node.children[child_index];

            // If this node is marked as a resource, update its methods and guards
            if let ResourceType::Resource = current_node.kind {
                if let Some(detail) = detail_registry.get(&current_node.full_path) {
                    update_unique(&mut current_node.methods, &detail.methods);
                    update_unique(&mut current_node.guards, &detail.guards);
                }
            }
        }
    }

    *get_registry().lock().unwrap() = root;
}

fn update_unique<T: Clone + PartialEq>(existing: &mut Vec<T>, new_items: &[T]) {
    for item in new_items {
        if !existing.contains(item) {
            existing.push(item.clone());
        }
    }
}

pub(crate) fn register_pattern_detail(
    full_path: String,
    methods: Vec<Method>,
    guards: Vec<String>,
    is_resource: bool,
) {
    if !is_designated_thread() {
        return;
    }
    initialize_detail_registry();
    let mut reg = get_detail_registry().lock().unwrap();
    reg.entry(full_path)
        .and_modify(|d| {
            update_unique(&mut d.methods, &methods);
            update_unique(&mut d.guards, &guards);
            // If the existing entry was not a resource but the new one is, update the kind
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

pub fn introspection_report_as_text() -> String {
    let registry = get_registry();
    let node = registry.lock().unwrap();
    let report_items: Vec<IntrospectionReportItem> = (&*node).into();

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

pub fn introspection_report_as_json() -> String {
    let registry = get_registry();
    let node = registry.lock().unwrap();
    let report_items: Vec<IntrospectionReportItem> = (&*node).into();

    serde_json::to_string_pretty(&report_items).unwrap()
}
