use std::collections::HashMap;

pub(crate) struct Registry<R, A> {
    routes: HashMap<u32, R>,
    active: HashMap<u32, A>,
    send_ids: HashMap<u32, u32>,
}

impl<R, A> Registry<R, A> {
    pub(crate) fn new() -> Self {
        Self {
            routes: HashMap::new(),
            active: HashMap::new(),
            send_ids: HashMap::new(),
        }
    }

    pub(crate) fn insert(&mut self, request_id: u32, route: R, active: A) -> bool {
        if self.routes.contains_key(&request_id) || self.active.contains_key(&request_id) {
            return false;
        }
        self.routes.insert(request_id, route);
        self.active.insert(request_id, active);
        true
    }

    pub(crate) fn active(&self, request_id: u32) -> Option<&A> {
        self.active.get(&request_id)
    }

    pub(crate) fn contains(&self, request_id: u32) -> bool {
        self.routes.contains_key(&request_id) || self.active.contains_key(&request_id)
    }

    pub(crate) fn route_mut(&mut self, request_id: u32) -> Option<&mut R> {
        self.routes.get_mut(&request_id)
    }

    pub(crate) fn link_send_id(&mut self, send_id: u32, request_id: u32) {
        self.send_ids.insert(send_id, request_id);
    }

    pub(crate) fn request_for_send_id(&mut self, send_id: u32) -> Option<u32> {
        self.send_ids.remove(&send_id)
    }

    pub(crate) fn remove(&mut self, request_id: u32) -> (Option<R>, Option<A>) {
        self.send_ids.retain(|_, value| *value != request_id);
        (
            self.routes.remove(&request_id),
            self.active.remove(&request_id),
        )
    }

    pub(crate) fn fail_all(&mut self, mut fail: impl FnMut(&mut R)) {
        for route in self.routes.values_mut() {
            fail(route);
        }
        self.routes.clear();
        self.active.clear();
        self.send_ids.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correlates_send_ids_and_removes_all_request_state() {
        let mut registry = Registry::new();
        assert!(registry.insert(7, "route", "active"));
        registry.link_send_id(42, 7);

        assert_eq!(registry.request_for_send_id(42), Some(7));
        let removed = registry.remove(7);
        assert_eq!(removed, (Some("route"), Some("active")));
        assert!(registry.active(7).is_none());
        assert!(registry.route_mut(7).is_none());
    }

    #[test]
    fn refuses_to_overwrite_a_live_request() {
        let mut registry = Registry::new();
        assert!(registry.insert(1, "first", 10));
        assert!(!registry.insert(1, "second", 20));
        assert_eq!(registry.route_mut(1).map(|route| *route), Some("first"));
        assert_eq!(registry.active(1), Some(&10));
    }

    #[test]
    fn fails_and_clears_every_route() {
        let mut registry = Registry::new();
        assert!(registry.insert(1, 1, ()));
        assert!(registry.insert(2, 2, ()));
        let mut failed = Vec::new();
        registry.fail_all(|route| failed.push(*route));
        failed.sort_unstable();
        assert_eq!(failed, vec![1, 2]);
        assert!(registry.active(1).is_none());
    }
}
