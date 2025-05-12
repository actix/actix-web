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
    REGISTRY.get_or_init(|| Mutex::new(IntrospectionNode::new(ResourceType::App, "".into())));
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
    pub methods: Vec<Method>,
    pub guards: Vec<String>,
    pub children: Vec<IntrospectionNode>,
}

impl IntrospectionNode {
    pub fn new(kind: ResourceType, pattern: String) -> Self {
        IntrospectionNode {
            kind,
            pattern,
            methods: Vec::new(),
            guards: Vec::new(),
            children: Vec::new(),
        }
    }

    pub fn display(&self, indent: usize, parent_path: &str) {
        let full_path = if parent_path.is_empty() {
            self.pattern.clone()
        } else {
            format!(
                "{}/{}",
                parent_path.trim_end_matches('/'),
                self.pattern.trim_start_matches('/')
            )
        };

        if !self.methods.is_empty() || !self.guards.is_empty() {
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

            println!("{}{}{}{}", " ".repeat(indent), full_path, methods, guards);
        }

        for child in &self.children {
            child.display(indent, &full_path);
        }
    }
}

fn build_tree(node: &mut IntrospectionNode, rmap: &ResourceMap) {
    initialize_detail_registry();
    let detail_registry = get_detail_registry();
    if let Some(ref children) = rmap.nodes {
        for child_rc in children {
            let child = child_rc;
            let pat = child.pattern.pattern().unwrap_or("").to_string();
            let kind = if child.nodes.is_some() {
                ResourceType::Scope
            } else {
                ResourceType::Resource
            };
            let mut new_node = IntrospectionNode::new(kind, pat.clone());

            if let ResourceType::Resource = new_node.kind {
                if let Some(d) = detail_registry.lock().unwrap().get(&pat) {
                    new_node.methods = d.methods.clone();
                    new_node.guards = d.guards.clone();
                }
            }

            build_tree(&mut new_node, child);
            node.children.push(new_node);
        }
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

pub fn register_rmap(rmap: &ResourceMap) {
    if !is_designated_thread() {
        return;
    }

    initialize_registry();
    let mut root = IntrospectionNode::new(ResourceType::App, "".into());
    build_tree(&mut root, rmap);
    *get_registry().lock().unwrap() = root;

    // WIP. Display the introspection tree
    let reg = get_registry().lock().unwrap();
    reg.display(0, "");
}

fn update_unique<T: Clone + PartialEq>(existing: &mut Vec<T>, new_items: &[T]) {
    for item in new_items {
        if !existing.contains(item) {
            existing.push(item.clone());
        }
    }
}

pub fn register_pattern_detail(pattern: String, methods: Vec<Method>, guards: Vec<String>) {
    if !is_designated_thread() {
        return;
    }
    initialize_detail_registry();
    let mut reg = get_detail_registry().lock().unwrap();
    reg.entry(pattern)
        .and_modify(|d| {
            update_unique(&mut d.methods, &methods);
            update_unique(&mut d.guards, &guards);
        })
        .or_insert(RouteDetail { methods, guards });
}
