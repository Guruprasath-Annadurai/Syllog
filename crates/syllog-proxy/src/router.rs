//! Ordered route selection with explicit circuit state.

use std::collections::HashSet;

use crate::ModelRoute;

/// Ordered routing table with an in-memory circuit-breaker view.
#[derive(Debug, Default)]
pub struct Router {
    routes: Vec<ModelRoute>,
    unavailable: HashSet<String>,
}

impl Router {
    /// Creates a router whose first route is preferred and later routes are fallbacks.
    #[must_use]
    pub fn new(routes: Vec<ModelRoute>) -> Self {
        Self {
            routes,
            unavailable: HashSet::new(),
        }
    }

    /// Marks a route unavailable until explicitly restored.
    pub fn trip(&mut self, name: impl Into<String>) {
        self.unavailable.insert(name.into());
    }

    /// Restores a previously tripped route.
    pub fn restore(&mut self, name: &str) {
        self.unavailable.remove(name);
    }

    /// Selects the first route whose circuit is closed.
    #[must_use]
    pub fn select(&self) -> Option<&ModelRoute> {
        self.routes
            .iter()
            .find(|route| !self.unavailable.contains(&route.name))
    }
}

#[cfg(test)]
mod tests {
    use super::Router;
    use crate::ModelRoute;

    fn route(name: &str) -> ModelRoute {
        ModelRoute {
            name: name.into(),
            provider: "test".into(),
            model: name.into(),
        }
    }

    #[test]
    fn tripped_primary_uses_fallback_until_restored() {
        let mut router = Router::new(vec![route("primary"), route("fallback")]);
        router.trip("primary");
        assert_eq!(
            router.select().map(|route| route.name.as_str()),
            Some("fallback")
        );
        router.restore("primary");
        assert_eq!(
            router.select().map(|route| route.name.as_str()),
            Some("primary")
        );
    }
}
