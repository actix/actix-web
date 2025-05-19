use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock,
    },
    thread,
};

use crate::{http::Method, rmap::ResourceMap};

static REGISTRY: OnceLock<Mutex<IntrospectionNode>> = OnceLock::new();
static DETAIL_REGISTRY: OnceLock<Mutex<HashMap<String, RouteDetail>>> = OnceLock::new();
static DESIGNATED_THREAD: OnceLock<thread::ThreadId> = OnceLock::new();
static IS_INITIALIZED: AtomicBool = AtomicBool::new(false);

pub fn initialize_registry() {
    REGISTRY.get_or_init(|| {
        Mutex::new(IntrospectionNode::new(
            ResourceType::App,
            "".into(),
            "".into(),
        ))
    });
}

pub fn get_registry() -> &'static Mutex<IntrospectionNode> {
    REGISTRY.get().expect("Registry not initialized")
}

pub fn initialize_detail_registry() {
    DETAIL_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()));
}

pub fn get_detail_registry() -> &'static Mutex<HashMap<String, RouteDetail>> {
    DETAIL_REGISTRY
        .get()
        .expect("Detail registry not initialized")
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
    pub pattern: String,   // Local pattern
    pub full_path: String, // Full path
    pub methods: Vec<Method>,
    pub guards: Vec<String>,
    pub children: Vec<IntrospectionNode>,
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

    pub fn display(&self, indent: usize) -> String {
        let mut result = String::new();

        // Helper function to determine if a node should be highlighted
        let should_highlight =
            |methods: &Vec<Method>, guards: &Vec<String>| !methods.is_empty() || !guards.is_empty();

        // Add the full path for all nodes
        if !self.full_path.is_empty() {
            if should_highlight(&self.methods, &self.guards) {
                // Highlight full_path with yellow if it has methods or guards
                result.push_str(&format!(
                    "{}\x1b[1;33m{}\x1b[0m",
                    " ".repeat(indent),
                    self.full_path
                ));
            } else {
                result.push_str(&format!("{}{}", " ".repeat(indent), self.full_path));
            }
        }

        // Only add methods and guards for resource nodes
        if let ResourceType::Resource = self.kind {
            let methods = if self.methods.is_empty() {
                "".to_string()
            } else {
                format!(" Methods: {:?}", self.methods)
            };
            let guards = if self.guards.is_empty() {
                "".to_string()
            } else {
                format!(" Guards: {:?}", self.guards)
            };

            // Highlight final endpoints with ANSI codes for bold and green color
            result.push_str(&format!("\x1b[1;32m{}{}\x1b[0m\n", methods, guards));
        } else {
            // For non-resource nodes, just add a newline
            result.push('\n');
        }

        for child in &self.children {
            result.push_str(&child.display(indent + 2)); // Increase indent for children
        }

        result
    }
}

fn is_designated_thread() -> bool {
    let current_id = thread::current().id();
    DESIGNATED_THREAD.get_or_init(|| {
        IS_INITIALIZED.store(true, Ordering::SeqCst);
        current_id // Assign the first thread that calls this function
    });

    *DESIGNATED_THREAD.get().unwrap() == current_id
}

pub fn register_rmap(_rmap: &ResourceMap) {
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

    // Display the introspection tree
    let registry = get_registry().lock().unwrap();
    let tree_representation = registry.display(0);
    log::debug!(
        "Introspection Tree:\n{}",
        tree_representation.trim_matches('\n')
    );
}

fn update_unique<T: Clone + PartialEq>(existing: &mut Vec<T>, new_items: &[T]) {
    for item in new_items {
        if !existing.contains(item) {
            existing.push(item.clone());
        }
    }
}

pub fn register_pattern_detail(
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
