use std::ops::Deref;
use std::sync::Arc;

/// Application state
pub struct State<S>(Arc<S>);

impl<S> State<S> {
    pub fn new(state: S) -> State<S> {
        State(Arc::new(state))
    }

    pub fn get_ref(&self) -> &S {
        self.0.as_ref()
    }
}

impl<S> Deref for State<S> {
    type Target = S;

    fn deref(&self) -> &S {
        self.0.as_ref()
    }
}

impl<S> Clone for State<S> {
    fn clone(&self) -> State<S> {
        State(self.0.clone())
    }
}
