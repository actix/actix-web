use std::{
    rc::Rc,
    sync::{OnceLock, RwLock},
    thread,
};

use crate::rmap::ResourceMap;

/// Represents an HTTP resource registered for introspection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceIntrospection {
    /// HTTP method (e.g., "GET").
    pub method: String,
    /// Route path (e.g., "/api/v1/test").
    pub path: String,
}

/// A global registry of listed resources for introspection.
/// Only the designated thread can modify it.
static RESOURCE_REGISTRY: RwLock<Vec<ResourceIntrospection>> = RwLock::new(Vec::new());

/// Stores the thread ID of the designated thread (the first to call `process_introspection`).
/// Any other thread will immediately return without updating the global registry.
static DESIGNATED_THREAD: OnceLock<thread::ThreadId> = OnceLock::new();

/// Inserts a resource into the global registry, avoiding duplicates.
pub fn register_resource(resource: ResourceIntrospection) {
    let mut global = RESOURCE_REGISTRY.write().unwrap();
    if !global.iter().any(|r| r == &resource) {
        global.push(resource);
    }
}

/// Completes (updates) partial routes in the global registry whose path contains `marker`,
/// by applying the specified `prefix`.
pub fn complete_partial_routes_with_marker(marker: &str, prefix: &str) {
    let mut global = RESOURCE_REGISTRY.write().unwrap();

    let mut updated = Vec::new();
    let mut remaining = Vec::new();

    // Move all items out of the current registry.
    for resource in global.drain(..) {
        if resource.path.contains(marker) {
            // Build the full path by applying the prefix if needed.
            let full_path = if prefix.is_empty() {
                resource.path.clone()
            } else if prefix.ends_with('/') || resource.path.starts_with('/') {
                format!("{}{}", prefix, resource.path)
            } else {
                format!("{}/{}", prefix, resource.path)
            };

            let completed = ResourceIntrospection {
                method: resource.method,
                path: full_path,
            };

            // Add to `updated` if it's not already in there.
            if !updated.iter().any(|r| r == &completed) {
                updated.push(completed);
            }
        } else {
            // Keep this resource as-is.
            remaining.push(resource);
        }
    }

    // Merge updated items back with the remaining ones.
    remaining.extend(updated);
    *global = remaining;
}

/// Returns a **copy** of the global registry (safe to call from any thread).
pub fn get_registered_resources() -> Vec<ResourceIntrospection> {
    RESOURCE_REGISTRY.read().unwrap().clone()
}

/// Processes introspection data for routes and methods.
/// Only the **first thread** that calls this function (the "designated" one) may update
/// the global resource registry. Any other thread will immediately return without updating it.
///
/// # Parameters
/// - `rmap`: A resource map convertible to a vector of route strings.
/// - `rdef_methods`: A vector of `(sub_path, [methods])`.
///   - A tuple with an **empty** methods vector is treated as a "marker" (a partial route)
///     for which we try to deduce a prefix by finding `sub_path` in a route, then calling
///     `complete_partial_routes_with_marker`.
///   - A tuple with one or more methods registers a resource with `register_resource`.
pub fn process_introspection(rmap: Rc<ResourceMap>, rdef_methods: Vec<(String, Vec<String>)>) {
    // Determine the designated thread: if none is set yet, assign the current thread's ID.
    // This ensures that the first thread to call this function becomes the designated thread.
    let current_id = thread::current().id();
    DESIGNATED_THREAD.get_or_init(|| current_id);

    // If the current thread is not the designated one, return immediately.
    // This ensures that only the designated thread updates the global registry,
    // avoiding any interleaving or inconsistent updates from other threads.
    if *DESIGNATED_THREAD.get().unwrap() != current_id {
        return;
    }

    let rmap_vec = rmap.to_vec();

    // If there is no data, nothing to process.
    // Avoid unnecessary work.
    if rmap_vec.is_empty() && rdef_methods.is_empty() {
        return;
    }

    // Keep track of the deduced prefix for partial routes.
    let mut deduced_prefix: Option<String> = None;

    // 1. Handle "marker" entries (where methods is empty).
    for (sub_path, http_methods) in &rdef_methods {
        if http_methods.is_empty() {
            // Find any route that contains sub_path and use it to deduce a prefix.
            if let Some(route) = rmap_vec.iter().find(|r| r.contains(sub_path)) {
                if let Some(pos) = route.find(sub_path) {
                    let prefix = &route[..pos];
                    deduced_prefix = Some(prefix.to_string());
                    // Complete partial routes in the global registry using this prefix.
                    complete_partial_routes_with_marker(sub_path, prefix);
                }
            }
        }
    }

    // 2. Handle endpoint entries (where methods is non-empty).
    for (sub_path, http_methods) in &rdef_methods {
        if !http_methods.is_empty() {
            // Identify candidate routes that end with sub_path (or exactly match "/" if sub_path == "/").
            let candidates: Vec<&String> = if sub_path == "/" {
                rmap_vec.iter().filter(|r| r.as_str() == "/").collect()
            } else {
                rmap_vec.iter().filter(|r| r.ends_with(sub_path)).collect()
            };

            // If we found any candidates, pick the best match.
            if !candidates.is_empty() {
                let chosen = if let Some(prefix) = &deduced_prefix {
                    if !prefix.is_empty() {
                        candidates
                            .iter()
                            .find(|&&r| r.starts_with(prefix))
                            .cloned()
                            .or_else(|| candidates.iter().min_by_key(|&&r| r.len()).cloned())
                    } else {
                        candidates.iter().min_by_key(|&&r| r.len()).cloned()
                    }
                } else {
                    candidates.iter().min_by_key(|&&r| r.len()).cloned()
                };

                if let Some(full_route) = chosen {
                    // Register the endpoint in the global resource registry.
                    register_resource(ResourceIntrospection {
                        method: http_methods.join(","),
                        path: full_route.clone(),
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{num::NonZeroUsize, rc::Rc};

    use actix_router::ResourceDef;
    use tokio::sync::oneshot;

    use super::*;
    use crate::rmap::ResourceMap;

    /// Helper function to create a ResourceMap from a list of route strings.
    /// It creates a root ResourceMap with an empty prefix and adds each route as a leaf.
    fn create_resource_map(routes: Vec<&str>) -> Rc<ResourceMap> {
        // Create a root node with an empty prefix.
        let mut root = ResourceMap::new(ResourceDef::root_prefix(""));
        // For each route, create a ResourceDef and add it as a leaf (nested = None).
        for route in routes {
            let mut def = ResourceDef::new(route);
            root.add(&mut def, None);
        }
        Rc::new(root)
    }

    // Helper function to run the full introspection flow.
    // It processes introspection data for multiple blocks, each with a different set of routes and methods.
    fn run_full_introspection_flow() {
        // Block 1:
        // rmap_vec: ["/item/{id}"]
        // rdef_methods: []
        process_introspection(create_resource_map(vec!["/item/{id}"]), vec![]);

        // Block 2:
        // rmap_vec: ["/info"]
        // rdef_methods: []
        process_introspection(create_resource_map(vec!["/info"]), vec![]);

        // Block 3:
        // rmap_vec: ["/guarded"]
        // rdef_methods: []
        process_introspection(create_resource_map(vec!["/guarded"]), vec![]);

        // Block 4:
        // rmap_vec: ["/v1/item/{id}", "/v1/info", "/v1/guarded"]
        // rdef_methods: [("/item/{id}", ["GET"]), ("/info", ["POST"]), ("/guarded", ["UNKNOWN"])]
        process_introspection(
            create_resource_map(vec!["/v1/item/{id}", "/v1/info", "/v1/guarded"]),
            vec![
                ("/item/{id}".to_string(), vec!["GET".to_string()]),
                ("/info".to_string(), vec!["POST".to_string()]),
                ("/guarded".to_string(), vec!["UNKNOWN".to_string()]),
            ],
        );

        // Block 5:
        // rmap_vec: ["/hello"]
        // rdef_methods: []
        process_introspection(create_resource_map(vec!["/hello"]), vec![]);

        // Block 6:
        // rmap_vec: ["/v2/hello"]
        // rdef_methods: [("/hello", ["GET"])]
        process_introspection(
            create_resource_map(vec!["/v2/hello"]),
            vec![("/hello".to_string(), vec!["GET".to_string()])],
        );

        // Block 7:
        // rmap_vec: ["/api/v1/item/{id}", "/api/v1/info", "/api/v1/guarded", "/api/v2/hello"]
        // rdef_methods: [("/v1", []), ("/v2", [])]
        process_introspection(
            create_resource_map(vec![
                "/api/v1/item/{id}",
                "/api/v1/info",
                "/api/v1/guarded",
                "/api/v2/hello",
            ]),
            vec![("/v1".to_string(), vec![]), ("/v2".to_string(), vec![])],
        );

        // Block 8:
        // rmap_vec: ["/dashboard"]
        // rdef_methods: []
        process_introspection(create_resource_map(vec!["/dashboard"]), vec![]);

        // Block 9:
        // rmap_vec: ["/settings"]
        // rdef_methods: [("/settings", ["GET"]), ("/settings", ["POST"])]
        process_introspection(
            create_resource_map(vec!["/settings"]),
            vec![
                ("/settings".to_string(), vec!["GET".to_string()]),
                ("/settings".to_string(), vec!["POST".to_string()]),
            ],
        );

        // Block 10:
        // rmap_vec: ["/admin/dashboard", "/admin/settings"]
        // rdef_methods: [("/dashboard", ["GET"]), ("/settings", [])]
        process_introspection(
            create_resource_map(vec!["/admin/dashboard", "/admin/settings"]),
            vec![
                ("/dashboard".to_string(), vec!["GET".to_string()]),
                ("/settings".to_string(), vec![]),
            ],
        );

        // Block 11:
        // rmap_vec: ["/"]
        // rdef_methods: [("/", ["GET"]), ("/", ["POST"])]
        process_introspection(
            create_resource_map(vec!["/"]),
            vec![
                ("/".to_string(), vec!["GET".to_string()]),
                ("/".to_string(), vec!["POST".to_string()]),
            ],
        );

        // Block 12:
        // rmap_vec: ["/ping"]
        // rdef_methods: []
        process_introspection(create_resource_map(vec!["/ping"]), vec![]);

        // Block 13:
        // rmap_vec: ["/multi"]
        // rdef_methods: [("/multi", ["GET"]), ("/multi", ["POST"])]
        process_introspection(
            create_resource_map(vec!["/multi"]),
            vec![
                ("/multi".to_string(), vec!["GET".to_string()]),
                ("/multi".to_string(), vec!["POST".to_string()]),
            ],
        );

        // Block 14:
        // rmap_vec: ["/extra/ping", "/extra/multi"]
        // rdef_methods: [("/ping", ["GET"]), ("/multi", [])]
        process_introspection(
            create_resource_map(vec!["/extra/ping", "/extra/multi"]),
            vec![
                ("/ping".to_string(), vec!["GET".to_string()]),
                ("/multi".to_string(), vec![]),
            ],
        );

        // Block 15:
        // rmap_vec: ["/other_guard"]
        // rdef_methods: []
        process_introspection(create_resource_map(vec!["/other_guard"]), vec![]);

        // Block 16:
        // rmap_vec: ["/all_guard"]
        // rdef_methods: []
        process_introspection(create_resource_map(vec!["/all_guard"]), vec![]);

        // Block 17:
        // rmap_vec: ["/api/v1/item/{id}", "/api/v1/info", "/api/v1/guarded", "/api/v2/hello",
        //           "/admin/dashboard", "/admin/settings", "/", "/extra/ping", "/extra/multi",
        //           "/other_guard", "/all_guard"]
        // rdef_methods: [("/api", []), ("/admin", []), ("/", []), ("/extra", []),
        //               ("/other_guard", ["UNKNOWN"]), ("/all_guard", ["GET", "UNKNOWN", "POST"])]
        process_introspection(
            create_resource_map(vec![
                "/api/v1/item/{id}",
                "/api/v1/info",
                "/api/v1/guarded",
                "/api/v2/hello",
                "/admin/dashboard",
                "/admin/settings",
                "/",
                "/extra/ping",
                "/extra/multi",
                "/other_guard",
                "/all_guard",
            ]),
            vec![
                ("/api".to_string(), vec![]),
                ("/admin".to_string(), vec![]),
                ("/".to_string(), vec![]),
                ("/extra".to_string(), vec![]),
                ("/other_guard".to_string(), vec!["UNKNOWN".to_string()]),
                (
                    "/all_guard".to_string(),
                    vec!["GET".to_string(), "UNKNOWN".to_string(), "POST".to_string()],
                ),
            ],
        );
    }

    /// This test spawns multiple tasks that run the full introspection flow concurrently.
    /// Only the designated task (the first one to call process_introspection) updates the global registry,
    /// ensuring that the internal order remains consistent. Finally, we verify that get_registered_resources()
    /// returns the expected set of listed resources.
    /// Using a dedicated arbiter for each task ensures that the global registry is thread-safe.
    #[actix_rt::test]
    async fn test_introspection() {
        // Number of tasks to spawn.
        const NUM_TASKS: usize = 4;
        let mut completion_receivers = Vec::with_capacity(NUM_TASKS);

        // Check that the registry is initially empty.
        let registered_resources = get_registered_resources();

        assert_eq!(
            registered_resources.len(),
            0,
            "Expected 0 registered resources, found: {:?}",
            registered_resources
        );

        // Determine parallelism and max blocking threads.
        let parallelism = std::thread::available_parallelism().map_or(2, NonZeroUsize::get);
        let max_blocking_threads = std::cmp::max(512 / parallelism, 1);

        // Spawn tasks on the arbiter. Each task runs the full introspection flow and then signals completion.
        for _ in 0..NUM_TASKS {
            let (tx, rx) = oneshot::channel();
            // Create an Arbiter with a dedicated Tokio runtime.
            let arbiter = actix_rt::Arbiter::with_tokio_rt(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .max_blocking_threads(max_blocking_threads)
                    .build()
                    .unwrap()
            });
            // Spawn the task on the arbiter.
            arbiter.spawn(async move {
                run_full_introspection_flow();
                // Signal that this task has finished.
                let _ = tx.send(());
            });
            completion_receivers.push(rx);
        }

        // Wait for all spawned tasks to complete.
        for rx in completion_receivers {
            let _ = rx.await;
        }

        // After all blocks, we expect the final registry to contain 14 entries.
        let registered_resources = get_registered_resources();

        assert_eq!(
            registered_resources.len(),
            14,
            "Expected 14 registered resources, found: {:?}",
            registered_resources
        );

        // List of expected resources
        let expected_resources = vec![
            ResourceIntrospection {
                method: "GET".to_string(),
                path: "/api/v1/item/{id}".to_string(),
            },
            ResourceIntrospection {
                method: "POST".to_string(),
                path: "/api/v1/info".to_string(),
            },
            ResourceIntrospection {
                method: "UNKNOWN".to_string(),
                path: "/api/v1/guarded".to_string(),
            },
            ResourceIntrospection {
                method: "GET".to_string(),
                path: "/api/v2/hello".to_string(),
            },
            ResourceIntrospection {
                method: "GET".to_string(),
                path: "/admin/settings".to_string(),
            },
            ResourceIntrospection {
                method: "POST".to_string(),
                path: "/admin/settings".to_string(),
            },
            ResourceIntrospection {
                method: "GET".to_string(),
                path: "/admin/dashboard".to_string(),
            },
            ResourceIntrospection {
                method: "GET".to_string(),
                path: "/extra/multi".to_string(),
            },
            ResourceIntrospection {
                method: "POST".to_string(),
                path: "/extra/multi".to_string(),
            },
            ResourceIntrospection {
                method: "GET".to_string(),
                path: "/extra/ping".to_string(),
            },
            ResourceIntrospection {
                method: "GET".to_string(),
                path: "/".to_string(),
            },
            ResourceIntrospection {
                method: "POST".to_string(),
                path: "/".to_string(),
            },
            ResourceIntrospection {
                method: "UNKNOWN".to_string(),
                path: "/other_guard".to_string(),
            },
            ResourceIntrospection {
                method: "GET,UNKNOWN,POST".to_string(),
                path: "/all_guard".to_string(),
            },
        ];

        for exp in expected_resources {
            assert!(
                registered_resources.contains(&exp),
                "Expected resource not found: {:?}",
                exp
            );
        }
    }
}
