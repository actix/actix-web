use std::rc::Rc;
use std::sync::{OnceLock, RwLock};

use crate::dev::ResourceMap;

/// Represents an HTTP resource registered for introspection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceIntrospection {
    /// HTTP method (e.g., "GET")
    pub method: String,
    /// Path (e.g., "/api/v1/test")
    pub path: String,
}

/// Temporary registry for partial routes.
static TEMP_REGISTRY: OnceLock<RwLock<Vec<ResourceIntrospection>>> = OnceLock::new();
/// Final registry for complete routes.
static FINAL_REGISTRY: OnceLock<RwLock<Vec<ResourceIntrospection>>> = OnceLock::new();

fn get_temp_registry() -> &'static RwLock<Vec<ResourceIntrospection>> {
    TEMP_REGISTRY.get_or_init(|| RwLock::new(Vec::new()))
}

fn get_final_registry() -> &'static RwLock<Vec<ResourceIntrospection>> {
    FINAL_REGISTRY.get_or_init(|| RwLock::new(Vec::new()))
}

/// Registers a resource.
pub fn register_resource(resource: ResourceIntrospection, is_complete: bool) {
    let registry = if is_complete {
        get_final_registry()
    } else {
        get_temp_registry()
    };
    let mut reg = registry.write().expect("Failed to acquire lock");
    if !reg.iter().any(|r| r == &resource) {
        reg.push(resource);
    }
}

/// Completes (moves to the final registry) partial routes that match the given marker,
/// applying the prefix. Only affects routes whose path contains `marker`.
pub fn complete_partial_routes_with_marker(marker: &str, prefix: &str) {
    let temp_registry = get_temp_registry();
    let mut temp = temp_registry
        .write()
        .expect("Failed to acquire lock TEMP_REGISTRY");
    let final_registry = get_final_registry();
    let mut final_reg = final_registry
        .write()
        .expect("Failed to acquire lock FINAL_REGISTRY");

    let mut remaining = Vec::new();
    for resource in temp.drain(..) {
        if resource.path.contains(marker) {
            // Concatenate the prefix only if it is not already present.
            let full_path = if prefix.is_empty() {
                resource.path.clone()
            } else if prefix.ends_with("/") || resource.path.starts_with("/") {
                format!("{}{}", prefix, resource.path)
            } else {
                format!("{}/{}", prefix, resource.path)
            };
            let completed_resource = ResourceIntrospection {
                method: resource.method,
                path: full_path,
            };
            if !final_reg.iter().any(|r| r == &completed_resource) {
                final_reg.push(completed_resource);
            }
        } else {
            remaining.push(resource);
        }
    }
    *temp = remaining;
}

/// Returns the complete list of registered resources.
pub fn get_registered_resources() -> Vec<ResourceIntrospection> {
    let final_registry = get_final_registry();
    let final_reg = final_registry
        .read()
        .expect("Failed to acquire lock FINAL_REGISTRY");
    final_reg.clone()
}

/// Processes introspection data for routes and methods.
///
/// # Parameters
/// - `rmap`: A resource map that can be converted to a vector of full route strings.
/// - `rdef_methods`: A vector of tuples `(sub_path, [methods])`.
///   An entry with an empty methods vector is treated as a level marker, indicating that
///   routes registered in a lower level (in TEMP_REGISTRY) should be "completed" (moved to the final registry)
///   using the deduced prefix. For example, if a marker "/api" is found and the corresponding route is "/api/v1/item/{id}",
///   the deduced prefix will be "" (if the marker starts at the beginning) or a non-empty string indicating a higher level.
///
/// # Processing Steps
/// 1. **Marker Processing:**  
///    For each entry in `rdef_methods` with an empty methods vector, the function:
///      - Searches `rmap_vec` for a route that contains the `sub_path` (the marker).
///      - Deduces the prefix as the portion of the route before the marker.
///      - Calls `complete_partial_routes_with_marker`, moving all partial routes that contain the marker
///        from TEMP_REGISTRY to FINAL_REGISTRY, applying the deduced prefix.
///
/// 2. **Endpoint Registration:**  
///    For each entry in `rdef_methods` with assigned methods:
///      - If `sub_path` is "/", an exact match is used; otherwise, routes ending with `sub_path` are considered.
///      - Among the candidate routes from `rmap_vec`, the candidate that either starts with the deduced prefix
///        (if non-empty) or the shortest candidate (if at root level or no prefix was deduced) is selected.
///      - A single `ResourceIntrospection` is registered with the full route and all methods joined by commas.
///
/// Note: If multiple markers exist in the same block, only the last one processed (and stored in `deduced_prefix`)
/// is used for selecting endpoint candidates. Consider refactoring if independent processing per level is desired.
pub fn process_introspection(rmap: Rc<ResourceMap>, rdef_methods: Vec<(String, Vec<String>)>) {
    // Convert the ResourceMap to a vector for easier manipulation.
    let rmap_vec = rmap.to_vec();

    // If there are no routes or methods, there is nothing to introspect.
    if rmap_vec.is_empty() && rdef_methods.is_empty() {
        return;
    }

    // Variable to store the deduced prefix for this introspection (if there is a marker)
    let mut deduced_prefix: Option<String> = None;

    // First, check the markers (entries with empty methods).
    for (sub_path, http_methods) in rdef_methods.iter() {
        if http_methods.is_empty() {
            if let Some(r) = rmap_vec.iter().find(|r| r.contains(sub_path)) {
                if let Some(pos) = r.find(sub_path) {
                    let prefix = &r[..pos];
                    deduced_prefix = Some(prefix.to_string());
                    complete_partial_routes_with_marker(sub_path, prefix);
                }
            }
        }
    }

    // Then, process the endpoints with assigned methods.
    for (sub_path, http_methods) in rdef_methods.iter() {
        if !http_methods.is_empty() {
            // For the "/" subroute, do an exact match; otherwise, use ends_with.
            let candidates: Vec<&String> = if sub_path == "/" {
                rmap_vec.iter().filter(|r| r.as_str() == "/").collect()
            } else {
                rmap_vec.iter().filter(|r| r.ends_with(sub_path)).collect()
            };

            if !candidates.is_empty() {
                let chosen = if let Some(prefix) = &deduced_prefix {
                    if !prefix.is_empty() {
                        candidates
                            .iter()
                            .find(|&&r| r.starts_with(prefix))
                            .cloned()
                            .or_else(|| candidates.iter().min_by_key(|&&r| r.len()).cloned())
                    } else {
                        // Root level: if sub_path is "/" we already filtered by equality.
                        candidates.iter().min_by_key(|&&r| r.len()).cloned()
                    }
                } else {
                    candidates.iter().min_by_key(|&&r| r.len()).cloned()
                };
                if let Some(full_route) = chosen {
                    // Register a single entry with all methods joined.
                    register_resource(
                        ResourceIntrospection {
                            method: http_methods.join(","),
                            path: full_route.clone(),
                        },
                        deduced_prefix.is_some(), // Mark as complete if any marker was detected.
                    );
                }
            }
        }
    }
}
